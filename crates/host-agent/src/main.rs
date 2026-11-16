//! Runs AssettoServer for Muretto. See the library for what it does.
//!
//! Configuration, from the environment:
//!
//! | Variable | Default | |
//! | --- | --- | --- |
//! | `AGENT_TOKENS` | | Required. `name:token,…`; the name shows who saved a preset. Tokens are at least 16 characters. |
//! | `AGENT_ADDR` | `127.0.0.1:3200` | Where the API listens. Put TLS in front. |
//! | `AGENT_STATE_DIR` | `$SERVER_DIR/muretto` | Where presets are kept. |
//! | `SERVER_DIR` | `.` | AssettoServer's working directory, with `cfg/` and `content/`. |
//! | `SERVER_COMMAND` | `/app/AssettoServer` | |
//! | `SERVER_ARGS` | `--plugins-from-workdir` | Split on whitespace. |
//! | `SERVER_NAME` | | Put before each preset's name in the lobby. |
//! | `SERVER_TCP_PORT`, `SERVER_UDP_PORT` | `9600` | |
//! | `SERVER_HTTP_PORT` | `8081` | |
//! | `SERVER_ADMIN_PASSWORD` | | |
//! | `SERVER_REGISTER_TO_LOBBY` | `1` | `0` keeps the server out of the public list. |
//! | `SERVER_MAX_CLIENTS` | | The most drivers at once, whatever a preset allows. |

use std::process::ExitCode;

use muretto_host_agent::{App, config::Config, router};

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let addr = config.addr;
    let app = match App::open(config) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("Could not read the agent's state: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = app.resume().await {
        // The API still comes up, so a better preset can be activated.
        eprintln!("{error}");
    }

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Could not listen on {addr}: {error}");
            app.shutdown().await;
            return ExitCode::FAILURE;
        }
    };
    eprintln!("Listening on {addr}.");
    let served = axum::serve(listener, router(app.clone()))
        .with_graceful_shutdown(stop_signal())
        .await;
    app.shutdown().await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Ctrl-C, or the SIGTERM `docker stop` sends.
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM can be watched");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
