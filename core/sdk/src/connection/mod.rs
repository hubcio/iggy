use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker, ready},
    time::Duration,
};

use async_broadcast::{Receiver, Sender, broadcast};
use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use futures::{AsyncRead, AsyncWrite, future::poll_fn};
use iggy_binary_protocol::{BinaryClient, BinaryTransport, Client};
use iggy_common::{
    ClientState, Command, DiagnosticEvent, IggyDuration, IggyError, TcpClientConfig,
};
use tokio::{
    net::TcpStream,
    sync::Mutex as TokioMutex,
    time::{Sleep, sleep},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, error, info, trace, warn};

use crate::protocol::{Order, ProtocolCore, ProtocolCoreConfig, Response, TxBuf};

#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub pending_sends: usize,
    pub pending_receives: usize,
}

pub trait AsyncIO: AsyncWrite + AsyncRead + Unpin {}
impl<T: AsyncRead + AsyncWrite + Unpin> AsyncIO for T {}

pub trait Runtime: Send + Sync + Debug {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>);
}

#[derive(Debug)]
pub struct TokioRuntime {}

impl Runtime for TokioRuntime {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        tokio::spawn(future);
    }
}

#[derive(Debug)]
struct SendState {
    request_id: u64,
    encoded: BytesMut,
    offset: usize,
}

#[derive(Debug)]
pub struct ConnectionState {
    core: ProtocolCore,
    pub recv_waiters: HashMap<u64, Waker>,
    ready_responses: HashMap<u64, Result<Bytes, IggyError>>,
    driver: Option<Waker>,
    error: Option<IggyError>,
    pending_orders: VecDeque<Order>,
}

struct ConnectionDriver<S> {
    state: Arc<Mutex<ConnectionState>>,
    socket: Pin<Box<S>>,
    current_send: Option<TxBuf>,
    send_offset: usize,
    recv_buffer: BytesMut,
    wait_timer: Option<Pin<Box<Sleep>>>,
}

impl<S: AsyncIO + Send> ConnectionDriver<S> {
    fn new(state: Arc<Mutex<ConnectionState>>, socket: S) -> Self {
        Self {
            state,
            socket: Box::pin(socket),
            current_send: None,
            send_offset: 0,
            recv_buffer: BytesMut::with_capacity(4096),
            wait_timer: None,
        }
    }
}

impl<S: AsyncIO> Future for ConnectionDriver<S> {
    type Output = Result<(), io::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        {
            let mut st = self.state.lock().unwrap();
            st.driver = Some(cx.waker().clone());
        }

        let mut recv_scratch = vec![0u8; 4096];
        if let Some(t) = &mut self.wait_timer {
            if t.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.wait_timer = None;
        }

        let order = {
            let mut st = self.state.lock().unwrap();
            st.core.poll()
        };

        match order {
            Order::Wait(dur) => {
                self.wait_timer = Some(Box::pin(tokio::time::sleep(dur.get_duration())));
                return Poll::Pending;
            }
            Order::Transmit(tx) => {
                self.current_send = Some(tx);
                self.send_offset = 0;
            }
            Order::Error(e) => {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{e:?}"))));
            }
            Order::Noop | Order::Authenticate { .. } => {}
            Order::Connect => unreachable!("handled in Transport"),
        }

        let mut offset = self.send_offset;
        // TODO запихать обратно в случае неудачи
        if let Some(buf) = self.current_send.take() {
            while self.send_offset < buf.data.len() {
                let n = ready!(
                    self.socket
                        .as_mut()
                        .poll_write(cx, &buf.data[offset..])
                )?;
                offset += n;
            }
            ready!(self.socket.as_mut().poll_flush(cx))?;
            self.current_send = None;
        }

        loop {
            let n = match self.socket.as_mut().poll_read(cx, &mut recv_scratch) {
                Poll::Pending => break,
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                Poll::Ready(Ok(n)) => n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            };
            self.recv_buffer.extend_from_slice(&recv_scratch[..n]);
            self.process_incoming()?;
        }

        Poll::Pending
    }
}

impl<S: AsyncIO> ConnectionDriver<S> {
    fn process_incoming(&mut self) -> Result<(), io::Error> {
        loop {
            if self.recv_buffer.len() < 8 {
                break;
            }
            let status = u32::from_le_bytes(self.recv_buffer[0..4].try_into().unwrap());
            let length = u32::from_le_bytes(self.recv_buffer[4..8].try_into().unwrap());
            let total = 8usize + length as usize;
            if self.recv_buffer.len() < total {
                break;
            }

            self.recv_buffer.advance(8);
            let payload = if length > 0 {
                Bytes::copy_from_slice(&self.recv_buffer.split_to(length as usize))
            } else {
                Bytes::new()
            };

            let mut state = self.state.lock().unwrap();
            if let Some(request_id) = state.core.on_response(status, &payload) {
                let result = if status == 0 {
                    Ok(payload)
                } else {
                    Err(IggyError::from_code(status))
                };
                state.ready_responses.insert(request_id, result);
                if let Some(waker) = state.recv_waiters.remove(&request_id) {
                    waker.wake();
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct TokioTcpTransport {
    state: Arc<Mutex<ConnectionState>>,
    runtime: Arc<dyn Runtime>,
    config: Arc<TcpClientConfig>,
    socket_handle: Option<tokio::task::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl TokioTcpTransport {
    fn new(config: Arc<TcpClientConfig>, runtime: Arc<dyn Runtime>) -> Self {
        let core_config = ProtocolCoreConfig {
            max_retries: config.reconnection.max_retries,
            reestablish_after: config.reconnection.reestablish_after,
            auto_login: config.auto_login.clone(),
        };

        let state = Arc::new(Mutex::new(ConnectionState {
            core: ProtocolCore::new(core_config),
            recv_waiters: HashMap::new(),
            ready_responses: HashMap::new(),
            driver: None,
            error: None,
            pending_orders: VecDeque::new(),
        }));

        Self {
            state,
            runtime,
            config,
            socket_handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn run(&mut self) {
        {
            let mut state = self.state.lock().unwrap();
            let order = state.core.poll();
            if !matches!(order, Order::Connect | Order::Wait(_)) {
                debug!("Initial poll returned: {:?}", order);
            }
        }

        while !self.shutdown.load(Ordering::Relaxed) {
            let order = {
                let mut state = self.state.lock().unwrap();
                state.core.poll()
            };

            match order {
                Order::Connect => match TcpStream::connect(&self.config.server_address).await {
                    Ok(socket) => {
                        info!("Connected to {}", self.config.server_address);

                        {
                            let mut state = self.state.lock().unwrap();
                            state.core.on_connected();
                        }

                        let socket_compat = socket.compat();
                        let driver = ConnectionDriver::new(self.state.clone(), socket_compat);
                        let state_clone = self.state.clone();
                        let handle = tokio::spawn(async move {
                            if let Err(e) = driver.await {
                                error!("Driver error: {}", e);
                                let mut state = state_clone.lock().unwrap();
                                state.core.on_disconnected();
                            }
                        });

                        self.socket_handle = Some(handle);
                    }
                    Err(e) => {
                        error!("Connection failed: {}", e);
                        let mut state = self.state.lock().unwrap();
                        state.core.on_disconnected();
                    }
                },

                Order::Wait(duration) => {
                    tokio::select! {
                        _ = sleep(duration.get_duration()) => {}
                        _ = tokio::signal::ctrl_c() => {
                            self.shutdown.store(true, Ordering::Relaxed);
                        }
                    }
                }

                _ => {
                    sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    async fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);

        if let Some(handle) = self.socket_handle.take() {
            handle.abort();
        }
    }
}

#[derive(Debug)]
pub struct NewTcpClient {
    transport: Arc<TokioMutex<Option<TokioTcpTransport>>>,
    state: Arc<Mutex<ConnectionState>>,
    config: Arc<TcpClientConfig>,
    events: (Sender<DiagnosticEvent>, Receiver<DiagnosticEvent>),
    runtime: Arc<dyn Runtime>,
}

impl NewTcpClient {
    pub fn create(config: Arc<TcpClientConfig>) -> Result<Self, IggyError> {
        let runtime = Arc::new(TokioRuntime {});
        let transport = TokioTcpTransport::new(config.clone(), runtime.clone());
        let state = transport.state.clone();

        let (tx, rx) = broadcast(1000);

        Ok(Self {
            transport: Arc::new(TokioMutex::new(Some(transport))),
            state,
            config,
            events: (tx, rx),
            runtime,
        })
    }

    async fn send_raw(&self, code: u32, payload: Bytes) -> Result<Bytes, IggyError> {
        let request_id = {
            let mut state = self.state.lock().unwrap();
            let id = state.core.send(code, payload)?;

            if let Some(waker) = state.driver.take() {
                waker.wake();
            }

            id
        };

        poll_fn(move |cx| {
            let mut state = self.state.lock().unwrap();

            if let Some(ref error) = state.error {
                return Poll::Ready(Err(error.clone()));
            }

            if let Some(result) = state.ready_responses.remove(&request_id) {
                return Poll::Ready(result);
            }

            state
                .recv_waiters
                .entry(request_id)
                .or_insert_with(|| cx.waker().clone());

            Poll::Pending
        })
        .await
    }

    async fn wait_for_connection(&self) -> Result<(), IggyError> {
        poll_fn(|cx| {
            let state = self.state.lock().unwrap();

            if let Some(ref error) = state.error {
                return Poll::Ready(Err(error.clone()));
            }

            match state.core.state() {
                ClientState::Connected
                | ClientState::Authenticating
                | ClientState::Authenticated => Poll::Ready(Ok(())),
                ClientState::Disconnected if state.core.retry_count > 0 => {
                    Poll::Ready(Err(IggyError::CannotEstablishConnection))
                }
                _ => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        })
        .await
    }
}

#[async_trait]
impl Client for NewTcpClient {
    async fn connect(&self) -> Result<(), IggyError> {
        {
            let state = self.state.lock().unwrap();
            match state.core.state() {
                ClientState::Connected
                | ClientState::Authenticating
                | ClientState::Authenticated => {
                    debug!("Already connected");
                    return Ok(());
                }
                ClientState::Connecting => {
                    debug!("Already connecting");
                    return Ok(());
                }
                _ => {}
            }
        }

        let transport = self.transport.clone();
        let runtime = self.runtime.clone();

        runtime.spawn(Box::pin(async move {
            let mut transport_guard = transport.lock().await;
            if let Some(transport) = transport_guard.take() {
                let mut transport = transport;
                transport.run().await;
            }
        }));

        let timeout =
            tokio::time::timeout(Duration::from_secs(30), self.wait_for_connection()).await;

        match timeout {
            Ok(result) => result,
            Err(_) => Err(IggyError::ConnectionTimeout),
        }
    }

    async fn disconnect(&self) -> Result<(), IggyError> {
        let mut state = self.state.lock().unwrap();
        state.core.on_disconnected();
        // TODO add stop transport
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), IggyError> {
        // TODO add stop transport
        let mut state = self.state.lock().unwrap();
        state.core.shutdown();
        Ok(())
    }

    async fn subscribe_events(&self) -> Receiver<DiagnosticEvent> {
        self.events.1.clone()
    }
}

#[async_trait]
impl BinaryTransport for NewTcpClient {
    async fn get_state(&self) -> ClientState {
        let state = self.state.lock().unwrap();
        state.core.state()
    }

    async fn set_state(&self, _state: ClientState) {
        // State is managed by core, this is for compatibility
    }

    async fn publish_event(&self, event: DiagnosticEvent) {
        if let Err(error) = self.events.0.broadcast(event).await {
            error!("Failed to send a TCP diagnostic event: {error}");
        }
    }

    async fn send_with_response<T: Command>(&self, command: &T) -> Result<Bytes, IggyError> {
        command.validate()?;
        self.send_raw_with_response(command.code(), command.to_bytes())
            .await
    }

    async fn send_raw_with_response(&self, code: u32, payload: Bytes) -> Result<Bytes, IggyError> {
        self.send_raw(code, payload).await
    }

    fn get_heartbeat_interval(&self) -> IggyDuration {
        self.config.heartbeat_interval
    }
}

impl BinaryClient for NewTcpClient {}
