//! The way to the group server: a WebSocket, or for tests a server in
//! memory that keeps its rooms the same way.

use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

use muretto_group_protocol::{ClientMessage, MAX_FRAME, Outbox, PATH_PREFIX, Rooms, ServerMessage};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};

use super::network;
use crate::Result;

/// Opens connections to rooms on a group server.
pub trait Relay: Send + Sync {
    fn open(&self, room: &str) -> Result<Box<dyn Pipe>>;
}

/// One member's connection to a room. Dropping it leaves the room.
pub trait Pipe: Send {
    fn send(&mut self, message: &ClientMessage) -> Result<()>;
    /// The next message, waiting up to `wait`; `None` when nothing came.
    fn recv(&mut self, wait: Duration) -> Result<Option<ServerMessage>>;
}

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The server pings every 30 seconds; this long without a word from it, the
/// connection is taken for dead.
const SILENCE: Duration = Duration::from_secs(75);

/// A group server reached over the internet, at a `ws://` or `wss://` URL.
pub struct WebSocketRelay {
    pub url: String,
}

impl WebSocketRelay {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

struct Endpoint<'a> {
    secure: bool,
    host: &'a str,
    port: u16,
}

fn endpoint(url: &str) -> Option<Endpoint<'_>> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("wss://") {
        (true, rest)
    } else {
        (false, url.strip_prefix("ws://")?)
    };
    let authority = rest.split('/').next()?;
    let default = if secure { 443 } else { 80 };
    let (host, port) = match authority.rsplit_once(':') {
        // A colon inside an IPv6 address is not a port.
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => {
            (host, port.parse().ok()?)
        }
        _ => (authority, default),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (!host.is_empty()).then_some(Endpoint { secure, host, port })
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            Arc::new(
                rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .expect("ring supports the default TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth(),
            )
        })
        .clone()
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(stream) => stream,
            Stream::Tls(stream) => &stream.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(stream) => stream.read(buf),
            Stream::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(stream) => stream.write(buf),
            Stream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(stream) => stream.flush(),
            Stream::Tls(stream) => stream.flush(),
        }
    }
}

impl Relay for WebSocketRelay {
    fn open(&self, room: &str) -> Result<Box<dyn Pipe>> {
        let base = self.url.trim().trim_end_matches('/');
        let Some(endpoint) = endpoint(base) else {
            return Err(network(format!(
                "The group server address {base} is not a ws:// or wss:// address."
            )));
        };
        let unreachable =
            |error: io::Error| network(format!("Could not reach the group server: {error}."));
        let address = (endpoint.host, endpoint.port)
            .to_socket_addrs()
            .map_err(unreachable)?
            .next()
            .ok_or_else(|| network("The group server's address does not resolve."))?;
        let tcp = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(unreachable)?;
        tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
            .map_err(unreachable)?;
        tcp.set_write_timeout(Some(CONNECT_TIMEOUT))
            .map_err(unreachable)?;
        let _ = tcp.set_nodelay(true);
        let stream = if endpoint.secure {
            let name = rustls::pki_types::ServerName::try_from(endpoint.host.to_owned())
                .map_err(|_| network("The group server's name is not valid."))?;
            let connection = rustls::ClientConnection::new(tls_config(), name)
                .map_err(|error| network(format!("TLS: {error}")))?;
            Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp)))
        } else {
            Stream::Plain(tcp)
        };
        let url = format!("{base}{PATH_PREFIX}{room}");
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let (socket, _) =
            tungstenite::client::client_with_config(url.as_str(), stream, Some(config)).map_err(
                |error| network(format!("The group server refused the connection: {error}")),
            )?;
        Ok(Box::new(WebSocketPipe {
            socket,
            heard: Instant::now(),
        }))
    }
}

struct WebSocketPipe {
    socket: WebSocket<Stream>,
    heard: Instant,
}

fn lost(error: tungstenite::Error) -> crate::Error {
    network(format!("Lost the group server: {error}."))
}

impl Pipe for WebSocketPipe {
    fn send(&mut self, message: &ClientMessage) -> Result<()> {
        self.socket
            .send(Message::text(message.to_text()))
            .map_err(lost)
    }

    fn recv(&mut self, wait: Duration) -> Result<Option<ServerMessage>> {
        let wait = wait.max(Duration::from_millis(1));
        self.socket
            .get_ref()
            .tcp()
            .set_read_timeout(Some(wait))
            .map_err(|error| network(error.to_string()))?;
        match self.socket.read() {
            Ok(Message::Text(text)) => {
                self.heard = Instant::now();
                serde_json::from_str(text.as_str()).map(Some).map_err(|_| {
                    network("The group server sent something Muretto does not understand.")
                })
            }
            Ok(Message::Close(_)) => Err(network("The group server closed the connection.")),
            Ok(_) => {
                // Pings are answered by the socket itself.
                self.heard = Instant::now();
                Ok(None)
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                // Pongs that could not go out yet go now.
                let _ = self.socket.flush();
                if self.heard.elapsed() > SILENCE {
                    Err(network("The group server stopped answering."))
                } else {
                    Ok(None)
                }
            }
            Err(error) => Err(lost(error)),
        }
    }
}

impl Drop for WebSocketPipe {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}

struct Queue(mpsc::SyncSender<ServerMessage>);

impl Outbox for Queue {
    fn push(&self, message: ServerMessage) -> bool {
        self.0.try_send(message).is_ok()
    }
}

/// A group server in memory, keeping rooms exactly as the real one does.
/// For tests and screenshots: every [`Relay`] clone shares the same rooms.
#[derive(Clone, Default)]
pub struct Memory {
    rooms: Arc<Mutex<Rooms<Queue>>>,
    /// While set, connecting fails as if the server were down.
    pub down: Arc<AtomicBool>,
}

impl Memory {
    /// How many members are in rooms.
    pub fn members(&self) -> usize {
        self.rooms.lock().unwrap().members()
    }
}

impl Relay for Memory {
    fn open(&self, room: &str) -> Result<Box<dyn Pipe>> {
        if self.down.load(Ordering::SeqCst) {
            return Err(network("Could not reach the group server."));
        }
        let (sender, receiver) = mpsc::sync_channel(64);
        let id = self
            .rooms
            .lock()
            .unwrap()
            .join(room, Queue(sender))
            .map_err(|refusal| network(refusal.to_string()))?;
        Ok(Box::new(MemoryPipe {
            rooms: self.rooms.clone(),
            room: room.to_owned(),
            id,
            receiver,
        }))
    }
}

struct MemoryPipe {
    rooms: Arc<Mutex<Rooms<Queue>>>,
    room: String,
    id: u64,
    receiver: mpsc::Receiver<ServerMessage>,
}

impl Pipe for MemoryPipe {
    fn send(&mut self, message: &ClientMessage) -> Result<()> {
        let ClientMessage::State { data } = message.clone();
        self.rooms
            .lock()
            .unwrap()
            .set_state(&self.room, self.id, data)
            .map_err(|refusal| network(refusal.to_string()))
    }

    fn recv(&mut self, wait: Duration) -> Result<Option<ServerMessage>> {
        match self.receiver.recv_timeout(wait) {
            Ok(message) => Ok(Some(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(network("The group server dropped the connection."))
            }
        }
    }
}

impl Drop for MemoryPipe {
    fn drop(&mut self) {
        if let Ok(mut rooms) = self.rooms.lock() {
            rooms.leave(&self.room, self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_are_read() {
        let read = |url| endpoint(url).map(|e| (e.secure, e.host.to_owned(), e.port));
        assert_eq!(
            read("wss://groups.example.org"),
            Some((true, "groups.example.org".into(), 443))
        );
        assert_eq!(
            read("ws://127.0.0.1:3100/prefix"),
            Some((false, "127.0.0.1".into(), 3100))
        );
        assert_eq!(read("ws://[::1]:3100"), Some((false, "::1".into(), 3100)));
        assert_eq!(read("wss://[::1]"), Some((true, "::1".into(), 443)));
        assert_eq!(read("https://example.org"), None);
        assert_eq!(read("ws://"), None);
    }
}
