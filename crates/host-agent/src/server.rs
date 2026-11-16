//! AssettoServer as a child process: started, stopped, and started again
//! when it dies on its own.

use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use muretto_host_protocol::ServerState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::store::now;

/// How long the server gets to stop after being asked before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(10);
/// The wait before restarting a crashed server; doubled for each crash in a
/// row, up to [`MAX_BACKOFF`].
const BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// A server that ran this long before crashing starts a new count.
const STABLE: Duration = Duration::from_secs(60);

/// Starts and stops the server. Cheap to clone; every clone drives the same
/// process.
#[derive(Clone)]
pub struct Supervisor {
    commands: mpsc::Sender<Request>,
    state: Arc<Mutex<ServerState>>,
}

enum Request {
    /// Stop the server if it runs, then start it.
    Restart(oneshot::Sender<Result<(), String>>),
    Stop(oneshot::Sender<()>),
}

struct Exited {
    generation: u64,
    status: Option<ExitStatus>,
}

impl Supervisor {
    pub fn new(program: PathBuf, args: Vec<String>, dir: PathBuf) -> Self {
        Self::with_grace(program, args, dir, STOP_GRACE)
    }

    fn with_grace(program: PathBuf, args: Vec<String>, dir: PathBuf, grace: Duration) -> Self {
        let (commands, receiver) = mpsc::channel(8);
        let state = Arc::new(Mutex::new(ServerState::Idle));
        let actor = Actor {
            program,
            args,
            dir,
            grace,
            state: state.clone(),
            running: None,
            generation: 0,
            restarts: 0,
            restart_at: None,
        };
        tokio::spawn(actor.run(receiver));
        Self { commands, state }
    }

    pub fn state(&self) -> ServerState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stops the server if it runs, then starts it. Fails if it cannot be
    /// started at all; a server that starts and then dies is restarted.
    pub async fn restart(&self) -> Result<(), String> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Request::Restart(reply))
            .await
            .map_err(|_| "The supervisor is gone.".to_owned())?;
        answer
            .await
            .map_err(|_| "The supervisor is gone.".to_owned())?
    }

    pub async fn stop(&self) {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Request::Stop(reply)).await.is_ok() {
            let _ = answer.await;
        }
    }
}

struct Running {
    generation: u64,
    started: Instant,
    stop: oneshot::Sender<()>,
}

struct Actor {
    program: PathBuf,
    args: Vec<String>,
    dir: PathBuf,
    grace: Duration,
    state: Arc<Mutex<ServerState>>,
    running: Option<Running>,
    generation: u64,
    /// Crashes in a row.
    restarts: u32,
    restart_at: Option<tokio::time::Instant>,
}

impl Actor {
    async fn run(mut self, mut commands: mpsc::Receiver<Request>) {
        let (exits_tx, mut exits) = mpsc::channel::<Exited>(8);
        loop {
            let restart_at = self.restart_at;
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Request::Restart(reply)) => {
                        self.stop(&mut exits).await;
                        self.restarts = 0;
                        let _ = reply.send(self.start(&exits_tx));
                    }
                    Some(Request::Stop(reply)) => {
                        self.stop(&mut exits).await;
                        let _ = reply.send(());
                    }
                    None => {
                        self.stop(&mut exits).await;
                        return;
                    }
                },
                Some(exit) = exits.recv() => self.crashed(exit),
                _ = sleep_until(restart_at), if restart_at.is_some() => {
                    self.restart_at = None;
                    if let Err(error) = self.start(&exits_tx) {
                        eprintln!("{error}");
                        self.crashed(Exited { generation: self.generation, status: None });
                    }
                }
            }
        }
    }

    fn set(&self, state: ServerState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    fn start(&mut self, exits: &mpsc::Sender<Exited>) -> Result<(), String> {
        self.restart_at = None;
        let child = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("Could not start {}: {error}", self.program.display()))?;
        self.generation += 1;
        let (stop, stopped) = oneshot::channel();
        tokio::spawn(watch(
            child,
            self.generation,
            self.grace,
            stopped,
            exits.clone(),
        ));
        self.running = Some(Running {
            generation: self.generation,
            started: Instant::now(),
            stop,
        });
        self.set(ServerState::Running { since: now() });
        Ok(())
    }

    /// Stops the server if it runs, and cancels a pending restart.
    async fn stop(&mut self, exits: &mut mpsc::Receiver<Exited>) {
        self.restart_at = None;
        if let Some(running) = self.running.take() {
            let _ = running.stop.send(());
            while let Some(exit) = exits.recv().await {
                if exit.generation == running.generation {
                    break;
                }
            }
        }
        self.set(ServerState::Idle);
    }

    fn crashed(&mut self, exit: Exited) {
        // An exit from a server already stopped on purpose is old news.
        let started = match &self.running {
            Some(running) if running.generation == exit.generation => running.started,
            None if exit.status.is_none() => Instant::now(),
            _ => return,
        };
        self.running = None;
        if started.elapsed() >= STABLE {
            self.restarts = 0;
        }
        self.restarts += 1;
        let exit_code = exit.status.and_then(|status| status.code());
        eprintln!(
            "The server exited ({exit_code:?}); restart {} follows.",
            self.restarts
        );
        let backoff = BACKOFF
            .saturating_mul(1 << (self.restarts - 1).min(16))
            .min(MAX_BACKOFF);
        self.restart_at = Some(tokio::time::Instant::now() + backoff);
        self.set(ServerState::Crashed {
            exit_code,
            restarts: self.restarts,
        });
    }
}

async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Owns the child: reports when it exits, and stops it when asked.
async fn watch(
    mut child: Child,
    generation: u64,
    grace: Duration,
    mut stop: oneshot::Receiver<()>,
    exits: mpsc::Sender<Exited>,
) {
    let exited = tokio::select! {
        status = child.wait() => Some(status),
        _ = &mut stop => None,
    };
    let status = match exited {
        Some(status) => status.ok(),
        None => terminate(&mut child, grace).await,
    };
    let _ = exits.send(Exited { generation, status }).await;
}

/// Asks the child to stop, then kills it if it has not after `grace`.
async fn terminate(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: plain FFI call; the pid is our child, which has not been
        // reaped yet, so it cannot belong to another process.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
        if let Ok(status) = tokio::time::timeout(grace, child.wait()).await {
            return status.ok();
        }
    }
    let _ = child.start_kill();
    child.wait().await.ok()
}

/// How many drivers the server on `port` reports, from AssettoServer's
/// `/INFO`. `None` when it does not answer.
pub async fn players(port: u16) -> Option<u32> {
    let ask = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        stream
            .write_all(b"GET /INFO HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .await
            .ok()?;
        let mut reply = Vec::new();
        stream.take(256 * 1024).read_to_end(&mut reply).await.ok()?;
        let at = reply.windows(4).position(|w| w == b"\r\n\r\n")?;
        let info: serde_json::Value = serde_json::from_slice(&reply[at + 4..]).ok()?;
        u32::try_from(info.get("clients")?.as_u64()?).ok()
    };
    tokio::time::timeout(Duration::from_secs(2), ask)
        .await
        .ok()?
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Supervisor {
        Supervisor::new(
            "/bin/sh".into(),
            vec!["-c".into(), script.into()],
            std::env::temp_dir(),
        )
    }

    #[tokio::test]
    async fn starts_and_stops() {
        let server = sh("exec sleep 60");
        assert_eq!(server.state(), ServerState::Idle);
        server.restart().await.unwrap();
        assert!(matches!(server.state(), ServerState::Running { .. }));
        server.restart().await.unwrap();
        assert!(matches!(server.state(), ServerState::Running { .. }));
        server.stop().await;
        assert_eq!(server.state(), ServerState::Idle);
    }

    #[tokio::test]
    async fn a_server_ignoring_sigterm_is_killed() {
        let server = Supervisor::with_grace(
            "/bin/sh".into(),
            vec![
                "-c".into(),
                "trap '' TERM; while :; do sleep 0.1; done".into(),
            ],
            std::env::temp_dir(),
            Duration::from_millis(300),
        );
        server.restart().await.unwrap();
        // Let the shell set its trap before it is asked to stop.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let asked = Instant::now();
        let stopped = tokio::time::timeout(Duration::from_secs(5), server.stop()).await;
        assert!(
            asked.elapsed() >= Duration::from_millis(300),
            "stopped by SIGTERM"
        );
        assert!(stopped.is_ok());
        assert_eq!(server.state(), ServerState::Idle);
    }

    #[tokio::test]
    async fn a_crash_is_reported_and_restarted() {
        let server = sh("exit 3");
        server.restart().await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(server.state(), ServerState::Crashed { .. }) {
            assert!(Instant::now() < deadline, "never crashed");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            server.state(),
            ServerState::Crashed {
                exit_code: Some(3),
                restarts: 1
            }
        );
        server.stop().await;
        assert_eq!(server.state(), ServerState::Idle);
    }

    #[tokio::test]
    async fn a_missing_program_fails_to_start() {
        let server = Supervisor::new(
            "/nonexistent/AssettoServer".into(),
            vec![],
            std::env::temp_dir(),
        );
        assert!(server.restart().await.is_err());
    }

    #[tokio::test]
    async fn no_server_no_players() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(players(port).await, None);
    }

    #[tokio::test]
    async fn players_from_info() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 1024];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"clients\":4,\"maxclients\":24}")
                .await
                .unwrap();
        });
        assert_eq!(players(port).await, Some(4));
    }
}
