use std::{
    collections::VecDeque, sync::Arc, time::{Duration, Instant}
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use iggy_common::{AutoLogin, ClientState, Credentials, IggyDuration, IggyError};
use tracing::{debug, info, warn};

const RESPONSE_HEADER_SIZE: usize = 8;

#[derive(Debug)]
pub struct ProtocolCoreConfig {
    pub auto_login: AutoLogin,
    pub reestablish_after: IggyDuration,
    pub max_retries: Option<u32>,
}

#[derive(Debug)]
pub enum Order {
    Connect,
    Wait(IggyDuration),
    Transmit(TxBuf),
    Authenticate { username: String, password: String },
    Noop,
    Error(IggyError),
}

#[derive(Debug, Clone)]
pub struct TxBuf {
    pub data: Bytes,
    pub request_id: u64,
}

#[derive(Debug)]
pub struct Response {
    pub status: u32,
    pub payload: Bytes,
}

#[derive(Debug)]
pub struct ProtocolCore {
    state: ClientState,
    config: ProtocolCoreConfig,
    last_connect_attempt: Option<Instant>,
    retry_count: u32,
    next_request_id: u64,
    pending_sends: VecDeque<(u32, Bytes, u64)>,
    sent_order: VecDeque<u64>,
    auth_pending: bool,
    auth_request_id: Option<u64>,
}

impl ProtocolCore {
    pub fn new(config: ProtocolCoreConfig) -> Self {
        Self {
            state: ClientState::Disconnected,
            config,
            last_connect_attempt: None,
            retry_count: 0,
            next_request_id: 1,
            pending_sends: VecDeque::new(),
            sent_order: VecDeque::new(),
            auth_pending: false,
            auth_request_id: None,
        }
    }

    pub fn state(&self) -> ClientState {
        self.state
    }

    pub fn poll(&mut self) -> Order {
        match self.state {
            ClientState::Disconnected => self.handle_disconnected(),
            ClientState::Connecting => Order::Noop,
            ClientState::Connected => self.handle_connected(),
            ClientState::Authenticating => Order::Noop,
            ClientState::Authenticated => self.poll_transmit(),
            ClientState::Shutdown => Order::Noop,
        }
    }

    fn handle_disconnected(&mut self) -> Order {
        if let Some(last_attempt) = self.last_connect_attempt {
            let elapsed = last_attempt.elapsed().as_micros() as u64;
            let interval = self.config.reestablish_after.as_micros();

            if elapsed < interval {
                let remaining = IggyDuration::from(interval - elapsed);
                return Order::Wait(remaining);
            }
        }

        if let Some(max_retries) = self.config.max_retries {
            if self.retry_count >= max_retries {
                return Order::Error(IggyError::MaxRetriesExceeded);
            }
        }

        self.retry_count += 1;
        self.last_connect_attempt = Some(Instant::now());
        self.state = ClientState::Connecting;

        debug!("Initiating connection (attempt {})", self.retry_count);
        Order::Connect
    }

    fn handle_connected(&mut self) -> Order {
        match &self.config.auto_login {
            AutoLogin::Disabled => {
                info!("Automatic sign-in is disabled.");
                self.state = ClientState::Authenticated;
            }
            AutoLogin::Enabled(credentials) => {
                if !self.auth_pending {
                    self.state = ClientState::Authenticating;
                    self.auth_pending = true;

                    match credentials {
                        Credentials::UsernamePassword(username, password) => {
                            let auth_payload = encode_auth(&username, &password);
                            let auth_id = self.queue_send(0x0A, auth_payload);
                            self.auth_request_id = Some(auth_id);

                            return self.poll_transmit();
                        }
                        _ => {
                            todo!("add PersonalAccessToken")
                        }
                    }

                }
            }
        }

        self.poll_transmit()
    }

    fn poll_transmit(&mut self) -> Order {
        if let Some((code, payload, request_id)) = self.pending_sends.pop_front() {
            let mut buf = BytesMut::new();
            let total_len = (payload.len() + 4) as u32;
            buf.put_u32_le(total_len);
            buf.put_u32_le(code);
            buf.put_slice(&payload);

            self.sent_order.push_back(request_id);

            Order::Transmit(TxBuf {
                data: buf.freeze(),
                request_id,
            })
        } else {
            Order::Noop
        }
    }

    pub fn send(&mut self, code: u32, payload: Bytes) -> Result<u64, IggyError> {
        match self.state {
            ClientState::Shutdown => Err(IggyError::ClientShutdown),
            ClientState::Disconnected | ClientState::Connecting => Err(IggyError::NotConnected),
            ClientState::Connected | ClientState::Authenticating => {
                Ok(self.queue_send(code, payload))
            }
            ClientState::Authenticated => Ok(self.queue_send(code, payload)),
        }
    }

    fn queue_send(&mut self, code: u32, payload: Bytes) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        self.pending_sends.push_back((code, payload, request_id));
        request_id
    }

    pub fn on_connected(&mut self) {
        debug!("Transport connected");
        self.state = ClientState::Connected;
        self.retry_count = 0;
    }

    pub fn on_disconnected(&mut self) {
        debug!("Transport disconnected");
        self.state = ClientState::Disconnected;
        self.auth_pending = false;
        self.auth_request_id = None;
        self.sent_order.clear();
    }

    pub fn on_response(&mut self, status: u32, _payload: &Bytes) -> Option<u64> {
        let request_id = self.sent_order.pop_front()?;

        if Some(request_id) == self.auth_request_id {
            if status == 0 {
                self.on_authenticated();
            } else {
                warn!("Authentication failed with status: {}", status);
                self.state = ClientState::Connected;
                self.auth_pending = false;
            }
            self.auth_request_id = None;
        }

        Some(request_id)
    }

    fn on_authenticated(&mut self) {
        debug!("Authentication successful");
        self.state = ClientState::Authenticated;
        self.auth_pending = false;
    }

    pub fn shutdown(&mut self) {
        self.state = ClientState::Shutdown;
    }
}

fn encode_auth(username: &str, password: &str) -> Bytes {
    let mut buf = BytesMut::new();
    buf.put_u32_le(username.len() as u32);
    buf.put_slice(username.as_bytes());
    buf.put_u32_le(password.len() as u32);
    buf.put_slice(password.as_bytes());
    buf.freeze()
}
