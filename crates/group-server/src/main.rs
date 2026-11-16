//! The server friends' Muretto groups meet on.
//!
//! Each group is a room named by a hash of its code. The server keeps the
//! latest encrypted state of each member, hands those to whoever joins, and
//! passes every change on. It cannot read the states and never learns a
//! code. See `muretto-group-protocol` for the messages.
//!
//! Opened in a browser, the server shows a page about Muretto with its
//! downloads (see `site`).
//!
//! Configuration, from the environment:
//!
//! | Variable | Default | |
//! | --- | --- | --- |
//! | `GROUP_ADDR` | `127.0.0.1:3100` | Where to listen. Put TLS in front. |
//! | `GROUP_MAX_CONNECTIONS` | `4096` | Connections beyond this are turned away. |

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use muretto_group_protocol::{ClientMessage, Outbox, Rooms, ServerMessage};
use tokio::sync::mpsc;

mod site;

/// How often the server pings each member, and how long it waits to hear
/// anything at all before it gives up on one.
const PING_EVERY: Duration = Duration::from_secs(30);
const SILENCE: Duration = Duration::from_secs(75);
/// How long a member may take to accept a message.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Messages a member may send in a burst, and how many more each second.
/// Muretto sends at most four a second.
const BURST: f64 = 20.;
const REFILL: f64 = 5.;
/// Messages waiting for one member before it counts as not keeping up.
const QUEUE: usize = 64;
/// The largest message a member may send: a state and its wrapping.
const MAX_INCOMING: usize = muretto_group_protocol::MAX_STATE + 256;

struct Member(mpsc::Sender<ServerMessage>);

impl Outbox for Member {
    fn push(&self, message: ServerMessage) -> bool {
        self.0.try_send(message).is_ok()
    }
}

struct Server {
    rooms: Mutex<Rooms<Member>>,
    connections: AtomicUsize,
    max_connections: usize,
}

impl Server {
    fn new(max_connections: usize) -> Arc<Self> {
        Arc::new(Self {
            rooms: Mutex::new(Rooms::default()),
            connections: AtomicUsize::new(0),
            max_connections,
        })
    }

    fn rooms(&self) -> std::sync::MutexGuard<'_, Rooms<Member>> {
        // The rooms hold no invariant a panic elsewhere could break halfway.
        self.rooms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Counts a connection for as long as it lives.
struct Counted(Arc<Server>);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

fn app(server: Arc<Server>) -> Router {
    Router::new()
        .route("/v1/{room}", get(upgrade))
        .route("/health", get(|| async { "ok" }))
        .merge(site::routes())
        .with_state(server)
}

async fn upgrade(
    State(server): State<Arc<Server>>,
    Path(room): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    if !muretto_group_protocol::valid_room(&room) {
        return (StatusCode::BAD_REQUEST, "Not a group room.").into_response();
    }
    let count = server.connections.fetch_add(1, Ordering::SeqCst);
    let counted = Counted(server.clone());
    if count >= server.max_connections {
        return (StatusCode::SERVICE_UNAVAILABLE, "The group server is full.").into_response();
    }
    ws.max_message_size(MAX_INCOMING)
        .max_frame_size(MAX_INCOMING)
        .on_upgrade(move |socket| serve(socket, counted, room))
}

async fn send(socket: &mut WebSocket, message: Message) -> bool {
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(message)).await,
        Ok(Ok(()))
    )
}

async fn refuse(socket: &mut WebSocket, message: impl Into<String>) {
    let text = ServerMessage::Error {
        message: message.into(),
    }
    .to_text();
    let _ = send(socket, Message::Text(text.into())).await;
    let _ = send(socket, Message::Close(None)).await;
}

async fn serve(mut socket: WebSocket, counted: Counted, room: String) {
    let server = counted.0.clone();
    let (outbox, mut inbox) = mpsc::channel(QUEUE);
    let joined = server.rooms().join(&room, Member(outbox));
    let id = match joined {
        Ok(id) => id,
        Err(refusal) => return refuse(&mut socket, refusal.to_string()).await,
    };

    let mut tokens = BURST;
    let mut refilled = Instant::now();
    let mut heard = Instant::now();
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    loop {
        tokio::select! {
            message = inbox.recv() => {
                // The room let go of this member: it fell too far behind.
                let Some(message) = message else {
                    refuse(&mut socket, "You fell behind; reconnect.").await;
                    break;
                };
                if !send(&mut socket, Message::Text(message.to_text().into())).await {
                    break;
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENCE || !send(&mut socket, Message::Ping(Default::default())).await {
                    break;
                }
            }
            incoming = socket.recv() => {
                heard = Instant::now();
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        tokens = (tokens + refilled.elapsed().as_secs_f64() * REFILL).min(BURST);
                        refilled = Instant::now();
                        if tokens < 1. {
                            refuse(&mut socket, "Too many messages.").await;
                            break;
                        }
                        tokens -= 1.;
                        let ClientMessage::State { data } = match serde_json::from_str(&text) {
                            Ok(message) => message,
                            Err(_) => {
                                refuse(&mut socket, "Not a group message.").await;
                                break;
                            }
                        };
                        let kept = server.rooms().set_state(&room, id, data);
                        if let Err(refusal) = kept {
                            refuse(&mut socket, refusal.to_string()).await;
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(_))) => {
                        refuse(&mut socket, "Not a group message.").await;
                        break;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                }
            }
        }
    }
    server.rooms().leave(&room, id);
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let address = std::env::var("GROUP_ADDR").unwrap_or_else(|_| "127.0.0.1:3100".to_owned());
    let max_connections = std::env::var("GROUP_MAX_CONNECTIONS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(4096);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Muretto group server listening on {address}");
    axum::serve(listener, app(Server::new(max_connections)))
        .with_graceful_shutdown(stopped())
        .await
}

/// Waits for Ctrl+C, or for the SIGTERM a container runtime sends to stop
/// the server. In a container the server is process 1, which the kernel
/// never stops by default, so SIGTERM has to be handled here.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM can be handled");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    use super::*;

    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn start(max_connections: usize) -> (std::net::SocketAddr, Arc<Server>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = Server::new(max_connections);
        tokio::spawn(axum::serve(listener, app(server.clone())).into_future());
        (address, server)
    }

    async fn connect(address: std::net::SocketAddr, room: &str) -> Client {
        tokio_tungstenite::connect_async(format!("ws://{address}/v1/{room}"))
            .await
            .unwrap()
            .0
    }

    async fn next(client: &mut Client) -> ServerMessage {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), client.next())
                .await
                .expect("the server answers")
                .expect("the connection is open")
                .unwrap();
            if let WsMessage::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    async fn state(client: &mut Client, data: &str) {
        let text = ClientMessage::State { data: data.into() }.to_text();
        client.send(WsMessage::Text(text.into())).await.unwrap();
    }

    const ROOM: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[tokio::test]
    async fn members_see_each_other_s_state_and_who_leaves() {
        let (address, server) = start(16).await;
        let mut alex = connect(address, ROOM).await;
        let ServerMessage::Welcome { you: alex_id, .. } = next(&mut alex).await else {
            panic!("welcome first");
        };
        state(&mut alex, "alex-1").await;

        let mut sam = connect(address, ROOM).await;
        let ServerMessage::Welcome {
            you: sam_id,
            members,
        } = next(&mut sam).await
        else {
            panic!("welcome first");
        };
        assert_eq!(members.len(), 1);
        assert_eq!(
            (members[0].id, members[0].data.as_str()),
            (alex_id, "alex-1")
        );

        state(&mut sam, "sam-1").await;
        assert_eq!(
            next(&mut alex).await,
            ServerMessage::State {
                id: sam_id,
                data: "sam-1".into()
            }
        );
        sam.close(None).await.unwrap();
        assert_eq!(next(&mut alex).await, ServerMessage::Left { id: sam_id });
        drop(alex);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(server.rooms().rooms(), 0);
        assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn garbage_and_floods_are_turned_away() {
        let (address, _) = start(16).await;
        let mut client = connect(address, ROOM).await;
        next(&mut client).await;
        client
            .send(WsMessage::Text(r#"{"type":"hello"}"#.into()))
            .await
            .unwrap();
        assert!(matches!(
            next(&mut client).await,
            ServerMessage::Error { .. }
        ));

        let mut client = connect(address, ROOM).await;
        next(&mut client).await;
        for _ in 0..40 {
            if client
                .send(WsMessage::Text(
                    ClientMessage::State { data: "x".into() }.to_text().into(),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
        assert_eq!(
            next(&mut client).await,
            ServerMessage::Error {
                message: "Too many messages.".into()
            }
        );

        let bad = tokio_tungstenite::connect_async(format!("ws://{address}/v1/nope")).await;
        assert!(bad.is_err(), "a malformed room is refused before upgrading");
    }

    #[tokio::test]
    async fn a_full_server_refuses_more_connections() {
        let (address, server) = start(1).await;
        let mut first = connect(address, ROOM).await;
        next(&mut first).await;
        assert!(
            tokio_tungstenite::connect_async(format!("ws://{address}/v1/{ROOM}"))
                .await
                .is_err()
        );
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_browser_gets_the_landing_page_and_what_it_shows() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        async fn get(address: std::net::SocketAddr, path: &str) -> (String, Vec<u8>) {
            let mut tcp = tokio::net::TcpStream::connect(address).await.unwrap();
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: muretto\r\nConnection: close\r\n\r\n");
            tcp.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            tcp.read_to_end(&mut response).await.unwrap();
            let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let head = String::from_utf8(response[..split].to_vec()).unwrap();
            (head.to_lowercase(), response[split + 4..].to_vec())
        }

        let (address, _) = start(16).await;
        let (head, body) = get(address, "/").await;
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(head.contains("content-type: text/html"), "{head}");
        let page = String::from_utf8(body).unwrap();
        for link in [
            "/releases/latest/download/muretto-linux-x86_64\"",
            "/releases/latest/download/muretto-windows-x86_64.exe\"",
            "https://github.com/8-prime/Muretto\"",
        ] {
            assert!(page.contains(link), "the page links {link}");
        }
        for (path, kind) in [
            ("/drive.png", "image/png"),
            ("/icon.svg", "image/svg+xml"),
            ("/display.ttf", "font/ttf"),
        ] {
            assert!(
                page.contains(&format!("\"{path}\"")),
                "the page uses {path}"
            );
            let (head, body) = get(address, path).await;
            assert!(head.starts_with("http/1.1 200"), "{path}: {head}");
            assert!(
                head.contains(&format!("content-type: {kind}")),
                "{path}: {head}"
            );
            assert!(!body.is_empty());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn muretto_s_own_client_meets_a_friend_through_the_server() {
        use muretto_core::group::{Code, Event, Group, Presence, Viewed, WebSocketRelay};
        use std::sync::Mutex as StdMutex;

        let (address, _) = start(16).await;
        let relay: Arc<dyn muretto_core::group::Relay> =
            Arc::new(WebSocketRelay::new(format!("ws://{address}")));
        let code = Code::random();
        let presence = |device: &str, name: &str, http_port: Option<u16>| Presence {
            device: device.into(),
            name: name.into(),
            viewing: http_port.map(|http_port| Viewed {
                ip: "10.0.0.1".into(),
                http_port,
            }),
        };
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let alex = Group::join(
            relay.clone(),
            code.clone(),
            presence("a", "Alex", Some(8081)),
            |_| {},
        );
        let _sam = Group::join(relay, code, presence("s", "Sam", None), {
            let seen = seen.clone();
            move |event| {
                if let Event::Members(members) = event {
                    *seen.lock().unwrap() =
                        members.into_iter().map(|member| member.presence).collect();
                }
            }
        });
        let wait_for = |want: Vec<Presence>| {
            let seen = seen.clone();
            async move {
                let until = Instant::now() + Duration::from_secs(10);
                while *seen.lock().unwrap() != want {
                    assert!(Instant::now() < until, "Sam never saw {want:?}");
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        wait_for(vec![presence("a", "Alex", Some(8081))]).await;
        alex.tell(presence("a", "Alex", Some(8082)));
        wait_for(vec![presence("a", "Alex", Some(8082))]).await;
        drop(alex);
        wait_for(vec![]).await;
    }
}
