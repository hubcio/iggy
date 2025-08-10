use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    io,
    ops::Deref,
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
use futures::{future::poll_fn, AsyncRead, AsyncWrite, FutureExt};
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

pub enum ClientCommand<S: AsyncIO> {
    Connect((tokio::sync::oneshot::Sender<()>, Pin<Box<S>>)),
    Disconnect(tokio::sync::oneshot::Sender<()>),
    Shutdown(tokio::sync::oneshot::Sender<()>),
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

// sans-io state, который будет находиться отдкельно
#[derive(Debug)]
pub struct ProtoConnectionState {
    pub core: ProtocolCore,
    driver: Option<Waker>,
    error: Option<IggyError>,
}

pub struct ConnectionInner<S: AsyncIO> {
    pub(crate) state: Mutex<State<S>>,
}

pub struct ConnectionRef<S>(Arc<ConnectionInner<S>>);

impl<S> Deref for ConnectionRef<S> {
    type Target = ConnectionInner<S>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct State<S: AsyncIO> {
    inner: ProtoConnectionState,
    driver: Option<Waker>,
    socket: Pin<Box<S>>,
    current_send: Option<TxBuf>,
    send_offset: usize,
    recv_buffer: BytesMut,
    wait_timer: Option<Pin<Box<Sleep>>>,
    send_buf: Mutex<VecDeque<(u32, Bytes)>>,
    ready_responses: HashMap<u64, Result<Bytes, IggyError>>,
    pub recv_waiters: HashMap<u64, Waker>,
    client_commands_rx: flume::Receiver<ClientCommand<S>>,
    config: Arc<TcpClientConfig>,
}

impl<S: AsyncIO> State<S> {
    fn drive_client_commands(&mut self, cx: &mut Context<'_>) -> io::Result<bool> {
        let fut = self.client_commands_rx.recv_async();
        loop {
            match Pin::new(&mut fut).poll(cx) {
                Poll::Pending => return Ok(false),
                Poll::Ready(Ok(cmd)) => {
                    match cmd {
                        ClientCommand::Connect((tx, socket)) => {
                            // TODO add core.poll
                            self.socket = socket;
                            tx.send(());
                        },
                        _ => {todo!("обработать")}
                    }
                }
                Poll::Ready(Err(e)) => return Err(e)
            }
        }
    }

    fn drive_timer(&mut self, cx: &mut Context<'_>) -> bool {
        if let Some(t) = &mut self.wait_timer {
            if t.as_mut().poll(cx).is_pending() {
                return false;
            }
            self.wait_timer = None;
        }
        true
    }

    fn drive_transmit(&mut self, cx: &mut Context<'_>) -> io::Result<bool> {
        let mut offset = self.send_offset;
        if let Some(buf) = self.current_send.take() {
            while self.send_offset < buf.data.len() {
                match self.socket.as_mut().poll_write(cx, &buf.data[offset..])? {
                    Poll::Ready(n) => {
                        offset += n;
                    }
                    Poll::Pending => return Ok(false),
                }
            }
            match self.socket.as_mut().poll_flush(cx)? {
                Poll::Pending => return Ok(false),
                Poll::Ready(n_) => {}
            }
            self.current_send = None;
        }
        Ok(true)
    }

    fn drive_receive(&mut self, cx: &mut Context<'_>) -> io::Result<bool> {
        let mut recv_scratch = vec![0u8; 4096];
        loop {
            let n = match self.socket.as_mut().poll_read(cx, &mut recv_scratch)? {
                Poll::Pending => return Ok(false),
                Poll::Ready(n) => n,
            };
            self.recv_buffer.extend_from_slice(&recv_scratch[..n]);
            self.process_incoming()?;
        }
    }

    // TODO перенкести в sans io ядро
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

            if let Some(request_id) = self.inner.core.on_response(status, &payload) {
                let result = if status == 0 {
                    Ok(payload)
                } else {
                    Err(IggyError::from_code(status))
                };
                self.ready_responses.insert(request_id, result);
                if let Some(waker) = self.recv_waiters.remove(&request_id) {
                    waker.wake();
                }
            }
        }
        Ok(())
    }
}

struct ConnectionDriver<S>(ConnectionRef<S>);

impl<S: AsyncIO + Send> ConnectionDriver<S> {
    // fn new(state: Arc<Mutex<ProtoConnectionState>>, socket: S) -> Self {
    //     Self {
    //         state,
    //         socket: Box::pin(socket),
    //         current_send: None,
    //         send_offset: 0,
    //         recv_buffer: BytesMut::with_capacity(4096),
    //         wait_timer: None,
    //         send_buf: Mutex::new(VecDeque::new())
    //     }
    // }
}

impl<S: AsyncIO> Future for ConnectionDriver<S> {
    type Output = Result<(), io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let st = &mut *self.0.state.lock().unwrap();

        let mut keep_going = st.drive_timer(cx);

        let order = st.inner.core.poll();

        match order {
            Order::Wait(dur) => {
                st.wait_timer = Some(Box::pin(tokio::time::sleep(dur.get_duration())));
                return Poll::Pending;
            }
            Order::Transmit(tx) => {
                st.current_send = Some(tx);
                st.send_offset = 0;
            }
            Order::Error(e) => {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{e:?}"))));
            }
            Order::Noop | Order::Authenticate { .. } => {}
            Order::Connect => {todo!("добавить вызов метода из state")}
        }

        keep_going |= st.drive_client_commands(cx)?;
        keep_going |= st.drive_transmit(cx)?;
        keep_going |= st.drive_receive(cx)?;

        if keep_going {
            cx.waker().wake_by_ref();
        } else {
            st.driver = Some(cx.waker().clone());
        }

        Poll::Pending
    }
}

#[derive(Debug)]
pub struct TokioTcpTransport {
    state: Arc<Mutex<ProtoConnectionState>>,
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

        let state = Arc::new(Mutex::new(ProtoConnectionState {
            core: ProtocolCore::new(core_config),
            // recv_waiters: HashMap::new(),
            // ready_responses: HashMap::new(),
            driver: None,
            error: None,
        }));

        Self {
            state,
            runtime,
            config,
            socket_handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    // async fn run(&mut self) {
    //     {
    //         let mut state = self.state.lock().unwrap();
    //         let order = state.core.poll();
    //         if !matches!(order, Order::Connect | Order::Wait(_)) {
    //             debug!("Initial poll returned: {:?}", order);
    //         }
    //     }

    //     while !self.shutdown.load(Ordering::Relaxed) {
    //         let order = {
    //             let mut state = self.state.lock().unwrap();
    //             state.core.poll()
    //         };

    //         match order {
    //             Order::Connect => match TcpStream::connect(&self.config.server_address).await {
    //                 Ok(socket) => {
    //                     info!("Connected to {}", self.config.server_address);

    //                     {
    //                         let mut state = self.state.lock().unwrap();
    //                         state.core.on_connected();
    //                     }

    //                     let socket_compat = socket.compat();
    //                     let driver = ConnectionDriver::new(self.state.clone(), socket_compat);
    //                     let state_clone = self.state.clone();
    //                     let handle = tokio::spawn(async move {
    //                         if let Err(e) = driver.await {
    //                             error!("Driver error: {}", e);
    //                             let mut state = state_clone.lock().unwrap();
    //                             state.core.on_disconnected();
    //                         }
    //                     });

    //                     self.socket_handle = Some(handle);
    //                 }
    //                 Err(e) => {
    //                     error!("Connection failed: {}", e);
    //                     let mut state = self.state.lock().unwrap();
    //                     state.core.on_disconnected();
    //                 }
    //             },

    //             Order::Wait(duration) => {
    //                 tokio::select! {
    //                     _ = sleep(duration.get_duration()) => {}
    //                     _ = tokio::signal::ctrl_c() => {
    //                         self.shutdown.store(true, Ordering::Relaxed);
    //                     }
    //                 }
    //             }

    //             _ => {
    //                 sleep(Duration::from_millis(10)).await;
    //             }
    //         }
    //     }
    // }

    async fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);

        if let Some(handle) = self.socket_handle.take() {
            handle.abort();
        }
    }
}

#[derive(Debug)]
pub struct NewTcpClient<S: AsyncIO> {
    transport: Arc<TokioMutex<Option<TokioTcpTransport>>>,
    state: Arc<Mutex<ProtoConnectionState>>,
    config: Arc<TcpClientConfig>,
    events: (Sender<DiagnosticEvent>, Receiver<DiagnosticEvent>),
    runtime: Arc<dyn Runtime>,
    client_command_tx: flume::Sender<ClientCommand<S>>,
}

impl<S: AsyncIO> NewTcpClient<S> {
    pub fn create(config: Arc<TcpClientConfig>) -> Result<Self, IggyError> {
        let runtime = Arc::new(TokioRuntime {});
        let transport = TokioTcpTransport::new(config.clone(), runtime.clone());
        let state = transport.state.clone();

        let (tx, rx) = broadcast(1000);
        let (client_tx, client_rx) = flume::unbounded::<ClientCommand>();

        Ok(Self {
            transport: Arc::new(TokioMutex::new(Some(transport))),
            state,
            config,
            events: (tx, rx),
            runtime,
            client_command_tx: client_tx,
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

    // async fn wait_for_connection(&self) -> Result<(), IggyError> {
    //     poll_fn(|cx| {
    //         let state = self.state.lock().unwrap();

    //         if let Some(ref error) = state.error {
    //             return Poll::Ready(Err(error.clone()));
    //         }

    //         match state.core.state() {
    //             ClientState::Connected
    //             | ClientState::Authenticating
    //             | ClientState::Authenticated => Poll::Ready(Ok(())),
    //             ClientState::Disconnected if state.core.retry_count > 0 => {
    //                 Poll::Ready(Err(IggyError::CannotEstablishConnection))
    //             }
    //             _ => {
    //                 cx.waker().wake_by_ref();
    //                 Poll::Pending
    //             }
    //         }
    //     })
    //     .await
    // }
}

#[async_trait]
impl<S: AsyncIO + Debug + Send + Sync> Client for NewTcpClient<S> {
    async fn connect(&self) -> Result<(), IggyError> {
        let socket = TcpStream::connect(&self.config.server_address).await?;
        let socket_compat = socket.compat();
        let socket = Box::pin(socket_compat);

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        self.client_command_tx
            .send(ClientCommand::Connect((tx, socket)))
            .map_err(|_| IggyError::ConnectionTimeout)?;
        rx.await.unwrap();
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), IggyError> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        self.client_command_tx
            .send(ClientCommand::Disconnect(tx))
            .map_err(|_| IggyError::ConnectionTimeout)?;
        rx.await.unwrap();
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), IggyError> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        self.client_command_tx
            .send(ClientCommand::Shutdown(tx))
            .map_err(|_| IggyError::ConnectionTimeout)?;
        rx.await.unwrap();
        Ok(())
    }

    async fn subscribe_events(&self) -> Receiver<DiagnosticEvent> {
        self.events.1.clone()
    }
}

#[async_trait]
impl<S: AsyncIO + Debug + Send + Sync> BinaryTransport for NewTcpClient<S> {
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

impl<S: AsyncIO + Debug + Send + Sync> BinaryClient for NewTcpClient<S> {}
