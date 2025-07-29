use std::{collections::{HashMap, VecDeque}, fmt::Debug, io, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Waker}, time::Duration};

use async_broadcast::{broadcast, Receiver, Sender};
use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use futures::{future::poll_fn, AsyncRead, AsyncWrite};
use iggy_binary_protocol::{BinaryClient, BinaryTransport, Client};
use iggy_common::{ClientState, Command, DiagnosticEvent, IggyDuration, IggyError, TcpClientConfig};
use tokio::{net::TcpStream, sync::Mutex as TokioMutex, time::{sleep, Sleep}};
use tracing::{debug, error, info, trace, warn};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::protocol::{Order, ProtocolCore, ProtocolCoreConfig, Response, TxBuf};

#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub pending_sends: usize,
    pub pending_receives: usize,
}

pub trait AsyncIO: AsyncWrite + AsyncRead + Unpin{}
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

impl<S: AsyncIO + Send> Future for ConnectionDriver<S> {
    type Output = Result<(), io::Error>;
    
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut work_done = false;
        let mut recv_scratch = vec![0u8; 4096];

        loop {
            if let Some(timer) = &mut self.wait_timer {
                match timer.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        self.wait_timer = None;
                        work_done = true;
                    }
                    Poll::Pending => {
                        if !work_done {
                            return Poll::Pending;
                        }
                        todo!("some work")
                    }
                }
            }

            let order = {
                let mut state = self.state.lock().unwrap();
                state.driver = Some(cx.waker().clone());

                // Get next order from queue or poll core
                state.pending_orders.pop_front()
                    .unwrap_or_else(|| state.core.poll())
            };

            match order {
                Order::Connect => {
                    error!("Connect order in driver - should be handled by transport");
                }
                Order::Wait(duration) => {
                    self.wait_timer = Some(Box::pin(sleep(duration.get_duration())));
                    work_done = true;
                    continue;
                }
                Order::Transmit(tx_buf) => {
                    self.current_send = Some(tx_buf);
                    self.send_offset = 0;
                    work_done = true;
                }
                Order::Authenticate { .. } => {
                    continue;
                }
                Order::Noop => {}
                Order::Error(e) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Core error: {:?}", e)
                    )));
                }
            }

            if let Some(send) = self.current_send.take() {
                let remaining = &send.data[self.send_offset..];
                if !remaining.is_empty() {
                    match self.socket.as_mut().poll_write(cx, remaining) {
                        Poll::Ready(Ok(n)) => {
                            self.send_offset += n;
                            work_done = true;
                            
                            if self.send_offset >= send.data.len() {                                
                                let mut state = self.state.lock().unwrap();
                                let next_order = state.core.poll();
                                if !matches!(next_order, Order::Noop) {
                                    state.pending_orders.push_back(next_order);
                                }
                            } else {
                                self.current_send = Some(send);
                            }
                        }
                        Poll::Ready(Err(e)) => {
                            let mut state = self.state.lock().unwrap();
                            state.core.on_disconnected();
                            state.error = Some(IggyError::IoError);
                            return Poll::Ready(Err(e));
                        }
                        Poll::Pending => {
                            self.current_send = Some(send);
                        }
                    }
                }
            }

            match self.socket.as_mut().poll_read(cx, &mut recv_scratch) {
                Poll::Ready(Ok(0)) => {
                    let mut state = self.state.lock().unwrap();
                    state.core.on_disconnected();
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed"
                    )));
                }
                Poll::Ready(Ok(n)) => {
                    work_done = true;

                    self.recv_buffer.extend_from_slice(&recv_scratch[..n]);

                    while self.recv_buffer.len() >= 8 {
                        let status = u32::from_le_bytes(
                            self.recv_buffer[0..4].try_into().unwrap()
                        );
                        let length = u32::from_le_bytes(
                            self.recv_buffer[4..8].try_into().unwrap()
                        );
                        
                        let total = 8 + length as usize;
                        if self.recv_buffer.len() < total {
                            break;
                        }
                        
                        self.recv_buffer.advance(8);
                        let payload = if length > 0 {
                            let data = self.recv_buffer.split_to(length as usize);
                            Bytes::from(data)
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
                        
                        let next_order = state.core.poll();
                        if !matches!(next_order, Order::Noop) {
                            state.pending_orders.push_back(next_order);
                        }
                    }
                }
                Poll::Ready(Err(e)) => {
                    let mut state = self.state.lock().unwrap();
                    state.core.on_disconnected();
                    state.error = Some(IggyError::IoError);
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => {}
            }
            
            if !work_done {
                let mut state = self.state.lock().unwrap();
                state.driver = Some(cx.waker().clone());
                return Poll::Pending;
            }

            work_done = false;
        }    
    }
}

#[derive(Debug)]
pub struct TokioTcpTransport {
    state: Arc<Mutex<ConnectionState>>,
    runtime: Arc<dyn Runtime>,
    config: Arc<TcpClientConfig>,
    socket_handle: Option<tokio::task::JoinHandle<()>>,
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
        }
    }

    async fn run(&mut self) {
        loop {
            let order = {
                let mut state = self.state.lock().unwrap();
                state.core.poll()
            };
            
            match order {
                Order::Connect => {
                    match TcpStream::connect(&self.config.server_address).await {
                        Ok(socket) => {
                            info!("Connected to {}", self.config.server_address);
                            
                            {
                                let mut state = self.state.lock().unwrap();
                                state.core.on_connected();
                            }
                            let compat_socket = socket.compat();
                            let driver = ConnectionDriver::new(self.state.clone(), compat_socket);
                            let handle = tokio::spawn(async move {
                                if let Err(e) = driver.await {
                                    error!("Driver error: {}", e);
                                }
                            });
                            
                            self.socket_handle = Some(handle);
                        }
                        Err(e) => {
                            error!("Connection failed: {}", e);
                            let mut state = self.state.lock().unwrap();
                            state.core.on_disconnected();
                        }
                    }
                }
                
                Order::Wait(duration) => {
                    sleep(duration.get_duration()).await;
                }
                
                _ => {
                    sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct TcpClient {
    transport: Arc<TokioMutex<Option<TokioTcpTransport>>>,
    state: Arc<Mutex<ConnectionState>>,
    config: Arc<TcpClientConfig>,
    events: (Sender<DiagnosticEvent>, Receiver<DiagnosticEvent>),
    runtime: Arc<dyn Runtime>,
}

impl TcpClient {
    pub fn create(config: Arc<TcpClientConfig>) -> Result<Self, IggyError> {
        let runtime = Arc::new(TokioRuntime{});
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
            
            state.recv_waiters.entry(request_id)
                .or_insert_with(|| cx.waker().clone());
            
            Poll::Pending
        }).await
    }
}

#[async_trait]
impl Client for TcpClient {
    async fn connect(&self) -> Result<(), IggyError> {
        let transport = self.transport.clone();
        tokio::spawn(async move {
            let mut transport = transport.lock().await;
            if let Some(transport) = transport.as_mut() {
                transport.run().await;
            }
        });
        Ok(())
    }
    
    async fn disconnect(&self) -> Result<(), IggyError> {
        let mut state = self.state.lock().unwrap();
        state.core.on_disconnected();
        Ok(())
    }
    
    async fn shutdown(&self) -> Result<(), IggyError> {
        let mut state = self.state.lock().unwrap();
        state.core.shutdown();
        Ok(())
    }
    
    async fn subscribe_events(&self) -> Receiver<DiagnosticEvent> {
        self.events.1.clone()
    }
}

#[async_trait]
impl BinaryTransport for TcpClient {
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
        self.send_raw_with_response(command.code(), command.to_bytes()).await
    }
    
    async fn send_raw_with_response(&self, code: u32, payload: Bytes) -> Result<Bytes, IggyError> {
        self.send_raw(code, payload).await
    }
    
    fn get_heartbeat_interval(&self) -> IggyDuration {
        self.config.heartbeat_interval
    }
}

impl BinaryClient for TcpClient {}
