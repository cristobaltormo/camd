use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};

use super::codec::{self, ConnectReturnCode, Malformed, Packet};

pub const CONNECT_TIMEOUT_S: u64 = 5;

pub enum Request {
    Publish(String, Vec<u8>),
    Disconnect,
}

#[derive(Debug)]
pub enum Failure {
    Refused(ConnectReturnCode),
    Io(std::io::Error),
    Protocol(Malformed),
    Closed,
    NoPingResponse,
    Timeout,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Refused(code) => write!(f, "connection refused: {code:?}"),
            Failure::Io(e) => write!(f, "{e}"),
            Failure::Protocol(e) => write!(f, "malformed packet from the broker: {e}"),
            Failure::Closed => write!(f, "connection closed by the broker"),
            Failure::NoPingResponse => write!(f, "no answer to keepalive ping"),
            Failure::Timeout => write!(f, "timed out"),
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Io(e)
    }
}

impl From<Malformed> for Failure {
    fn from(e: Malformed) -> Self {
        Failure::Protocol(e)
    }
}

pub struct Settings {
    pub host: String,
    pub port: u16,
    pub keep_alive: Duration,
    pub connect: codec::Connect,
    pub tls: Option<Arc<rustls::ClientConfig>>,
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

pub struct Session {
    stream: Box<dyn Stream>,
    buffer: Vec<u8>,
    keep_alive: Duration,
    last_sent: Instant,
    awaiting_ping: bool,
    next_id: u16,
}

impl Session {
    pub async fn open(settings: &Settings) -> Result<Session, Failure> {
        timeout(
            Duration::from_secs(CONNECT_TIMEOUT_S),
            Self::handshake(settings),
        )
        .await
        .map_err(|_| Failure::Timeout)?
    }

    async fn handshake(settings: &Settings) -> Result<Session, Failure> {
        let tcp = TcpStream::connect((settings.host.as_str(), settings.port)).await?;
        let _ = tcp.set_nodelay(true);
        let stream: Box<dyn Stream> = match &settings.tls {
            Some(config) => {
                let name = rustls::pki_types::ServerName::try_from(settings.host.clone())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
                let connector = tokio_rustls::TlsConnector::from(Arc::clone(config));
                Box::new(connector.connect(name, tcp).await?)
            }
            None => Box::new(tcp),
        };
        let mut session = Session {
            stream,
            buffer: Vec::with_capacity(512),
            keep_alive: settings.keep_alive,
            last_sent: Instant::now(),
            awaiting_ping: false,
            next_id: 0,
        };
        session.send(&codec::connect(&settings.connect)?).await?;
        loop {
            match session.read_packet().await? {
                Packet::ConnAck {
                    code: ConnectReturnCode::Success,
                    ..
                } => return Ok(session),
                Packet::ConnAck { code, .. } => return Err(Failure::Refused(code)),
                _ => {}
            }
        }
    }

    pub async fn send(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let write = async {
            self.stream.write_all(bytes).await?;
            self.stream.flush().await
        };
        timeout(self.keep_alive, write)
            .await
            .map_err(|_| Failure::Timeout)??;
        self.last_sent = Instant::now();
        Ok(())
    }

    pub async fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), Failure> {
        match codec::publish(topic, payload, true) {
            Ok(bytes) => self.send(&bytes).await,
            Err(_) => Ok(()),
        }
    }

    pub async fn subscribe(&mut self, topic: &str) -> Result<(), Failure> {
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        match codec::subscribe(self.next_id, topic) {
            Ok(bytes) => self.send(&bytes).await,
            Err(_) => Ok(()),
        }
    }

    pub async fn disconnect(mut self) {
        let _ = self.send(&codec::DISCONNECT).await;
        let _ = self.stream.shutdown().await;
    }

    fn buffered(&mut self) -> Result<Option<Packet>, Failure> {
        match codec::decode(&self.buffer)? {
            Some((packet, used)) => {
                self.buffer.drain(..used);
                Ok(Some(packet))
            }
            None => Ok(None),
        }
    }

    async fn fill(&mut self) -> Result<(), Failure> {
        if self.stream.read_buf(&mut self.buffer).await? == 0 {
            return Err(Failure::Closed);
        }
        Ok(())
    }

    async fn read_packet(&mut self) -> Result<Packet, Failure> {
        loop {
            if let Some(packet) = self.buffered()? {
                return Ok(packet);
            }
            self.fill().await?;
        }
    }

    pub async fn run(
        &mut self,
        requests: &mut mpsc::Receiver<Request>,
        on_message: &mut dyn FnMut(&str, &[u8]),
    ) -> Result<(), Failure> {
        loop {
            while let Some(packet) = self.buffered()? {
                self.answer(packet, on_message).await?;
            }
            let ping_at = self.last_sent + self.keep_alive;
            tokio::select! {
                request = requests.recv() => match request {
                    Some(Request::Publish(topic, payload)) => self.publish(&topic, &payload).await?,
                    Some(Request::Disconnect) | None => return Ok(()),
                },
                read = self.stream.read_buf(&mut self.buffer) => {
                    if read? == 0 {
                        return Err(Failure::Closed);
                    }
                }
                () = tokio::time::sleep_until(ping_at) => {
                    if self.awaiting_ping {
                        return Err(Failure::NoPingResponse);
                    }
                    self.send(&codec::PINGREQ).await?;
                    self.awaiting_ping = true;
                }
            }
        }
    }

    async fn answer(
        &mut self,
        packet: Packet,
        on_message: &mut dyn FnMut(&str, &[u8]),
    ) -> Result<(), Failure> {
        match packet {
            Packet::Publish {
                topic,
                payload,
                qos,
                id,
                ..
            } => {
                on_message(&topic, &payload);
                match (qos, id) {
                    (1, Some(id)) => self.send(&codec::puback(id)).await?,
                    (2, Some(id)) => self.send(&codec::pubrec(id)).await?,
                    _ => {}
                }
            }
            Packet::PubRel(id) => self.send(&codec::pubcomp(id)).await?,
            Packet::PingResp => self.awaiting_ping = false,
            _ => {}
        }
        Ok(())
    }
}
