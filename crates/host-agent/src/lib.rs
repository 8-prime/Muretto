//! Runs AssettoServer for Muretto.
//!
//! The agent keeps a library of server presets that anyone holding a token
//! can edit and activate, and the few files from each car and track that the
//! server reads. Activating a preset writes it into AssettoServer's `cfg/`
//! and restarts the server, which runs as the agent's child. See
//! `muretto-host-protocol` for the API.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use muretto_host_protocol::{
    ActivePreset, ApiError, PATH_PREFIX, ServerState, Status,
    content::{Installed, MAX_FILE, Manifest, SHA256_HEADER, UploadPlan, classify, valid_sha256},
    presets::{Activate, FieldError, Preset, PresetId, PresetSummary, StoredPreset},
};

pub mod config;
mod content;
mod render;
mod server;
mod store;

use config::Config;
use content::{Content, UploadError};
use server::Supervisor;
use store::{Expect, SaveError, Store};

pub struct App {
    config: Config,
    store: Mutex<Store>,
    content: Content,
    server: Supervisor,
    /// One activation at a time: each stops and starts the server.
    activation: tokio::sync::Mutex<()>,
}

/// Who a request comes from, by the name that goes with their token.
#[derive(Clone)]
struct User(String);

impl App {
    /// Reads the agent's state. Must run inside the Tokio runtime, which the
    /// server's supervisor runs on.
    pub fn open(config: Config) -> io::Result<Arc<Self>> {
        let store = Store::open(&config.state_dir)?;
        let content = Content::new(config.server_dir.clone());
        let server = Supervisor::new(
            config.command.clone(),
            config.args.clone(),
            config.server_dir.clone(),
        );
        Ok(Arc::new(Self {
            config,
            store: Mutex::new(store),
            content,
            server,
            activation: tokio::sync::Mutex::new(()),
        }))
    }

    /// Starts the server with the preset it last ran, if any. The config is
    /// written again first, so changes to the host's own settings apply.
    pub async fn resume(&self) -> Result<(), String> {
        let Some(active) = self.store().active().cloned() else {
            return Ok(());
        };
        let files = render::render(&active.preset, &self.config.host);
        files
            .write(&self.cfg_dir())
            .map_err(|error| format!("Could not write the server's config: {error}"))?;
        self.server.restart().await
    }

    pub async fn shutdown(&self) {
        self.server.stop().await;
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        // Every change to the store is written to disk before it is made in
        // memory, so a panic halfway leaves nothing inconsistent behind.
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cfg_dir(&self) -> std::path::PathBuf {
        self.config.server_dir.join("cfg")
    }

    async fn status(&self) -> Status {
        let active = {
            let store = self.store();
            store.active().map(|active| {
                let saved = store.get(active.id).map(|preset| preset.revision);
                ActivePreset {
                    id: active.id,
                    name: active.preset.name.clone(),
                    revision: active.revision,
                    changed_since: saved.is_some_and(|saved| saved != active.revision),
                }
            })
        };
        let state = self.server.state();
        let players = match state {
            ServerState::Running { .. } => server::players(self.config.host.http_port).await,
            _ => None,
        };
        Status {
            active,
            server: state,
            players,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/presets", get(list_presets))
        .route(
            "/presets/{id}",
            get(get_preset).put(put_preset).delete(delete_preset),
        )
        .route("/presets/{id}/activate", post(activate))
        .route("/content", get(installed))
        .route("/content/plan", post(plan))
        .route(
            "/content/files/{*path}",
            put(upload).layer(DefaultBodyLimit::disable()),
        )
        .layer(middleware::from_fn_with_state(app.clone(), authorise))
        .with_state(app);
    Router::new().nest(PATH_PREFIX, api)
}

async fn authorise(State(app): State<Arc<App>>, mut request: Request, next: Next) -> Response {
    let name = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| app.config.tokens.name(token.trim()));
    match name {
        Some(name) => {
            let user = User(name.to_owned());
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        None => fail(StatusCode::UNAUTHORIZED, "That token is not known here.").into_response(),
    }
}

/// A refused request.
struct Failure(StatusCode, ApiError);

fn fail(status: StatusCode, message: impl Into<String>) -> Failure {
    Failure(
        status,
        ApiError {
            message: message.into(),
            fields: Vec::new(),
        },
    )
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        eprintln!("{error}");
        fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("The host could not read or write its files: {error}"),
        )
    }
}

impl From<JsonRejection> for Failure {
    fn from(rejection: JsonRejection) -> Self {
        fail(rejection.status(), rejection.body_text())
    }
}

fn mistakes(message: &str, fields: Vec<FieldError>) -> Failure {
    Failure(
        StatusCode::UNPROCESSABLE_ENTITY,
        ApiError {
            message: message.to_owned(),
            fields,
        },
    )
}

fn preset_id(id: &str) -> Result<PresetId, Failure> {
    id.parse()
        .map_err(|_| fail(StatusCode::NOT_FOUND, "There is no such preset."))
}

/// Runs file work off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("blocking work does not panic")
}

async fn status(State(app): State<Arc<App>>) -> Json<Status> {
    Json(app.status().await)
}

async fn list_presets(State(app): State<Arc<App>>) -> Json<Vec<PresetSummary>> {
    let store = app.store();
    let active = store.active().map(|active| active.id);
    let mut list: Vec<_> = store
        .list()
        .map(|preset| preset.summary(Some(preset.id) == active))
        .collect();
    list.sort_by_cached_key(|summary| summary.name.to_lowercase());
    Json(list)
}

async fn get_preset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<StoredPreset>, Failure> {
    let id = preset_id(&id)?;
    app.store()
        .get(id)
        .cloned()
        .map(Json)
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "There is no such preset."))
}

/// `If-None-Match: *` or `If-Match: <revision>`, quoted or not.
fn expectation(headers: &HeaderMap) -> Result<Option<Expect>, Failure> {
    let read = |name| {
        headers
            .get(name)
            .map(|value| {
                value
                    .to_str()
                    .map(|v| v.trim().trim_matches('"').to_owned())
            })
            .transpose()
            .map_err(|_| fail(StatusCode::BAD_REQUEST, "That header is not text."))
    };
    if let Some(value) = read(header::IF_NONE_MATCH)? {
        return match value.as_str() {
            "*" => Ok(Some(Expect::Nothing)),
            _ => Err(fail(
                StatusCode::BAD_REQUEST,
                "If-None-Match takes only `*`.",
            )),
        };
    }
    match read(header::IF_MATCH)? {
        Some(value) => value
            .parse()
            .map(|revision| Some(Expect::Revision(revision)))
            .map_err(|_| fail(StatusCode::BAD_REQUEST, "If-Match takes a revision number.")),
        None => Ok(None),
    }
}

async fn put_preset(
    State(app): State<Arc<App>>,
    Extension(User(by)): Extension<User>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<Preset>, JsonRejection>,
) -> Result<Json<StoredPreset>, Failure> {
    let id = preset_id(&id)?;
    let expect = expectation(&headers)?;
    let Json(preset) = body?;
    let errors = preset.validate();
    if !errors.is_empty() {
        return Err(mistakes("The preset has mistakes.", errors));
    }
    let saved = app.store().save(id, preset, expect, &by);
    match saved {
        Ok(stored) => Ok(Json(stored)),
        Err(SaveError::Unconditional) => Err(fail(
            StatusCode::PRECONDITION_REQUIRED,
            "Send If-Match with the revision you edited, or If-None-Match: * to create.",
        )),
        Err(SaveError::Conflict(Some(revision))) => Err(fail(
            StatusCode::CONFLICT,
            format!("The preset was saved since you opened it; it is at revision {revision} now."),
        )),
        Err(SaveError::Conflict(None)) => {
            Err(fail(StatusCode::NOT_FOUND, "There is no such preset."))
        }
        Err(SaveError::Io(error)) => Err(error.into()),
    }
}

async fn delete_preset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let id = preset_id(&id)?;
    if app.store().delete(id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(fail(StatusCode::NOT_FOUND, "There is no such preset."))
    }
}

async fn activate(
    State(app): State<Arc<App>>,
    Extension(User(by)): Extension<User>,
    Path(id): Path<String>,
    body: Result<Json<Activate>, JsonRejection>,
) -> Result<Json<Status>, Failure> {
    let id = preset_id(&id)?;
    let Json(options) = body?;
    let _one_at_a_time = app.activation.lock().await;

    let stored = app
        .store()
        .get(id)
        .cloned()
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "There is no such preset."))?;
    let mut errors = stored.preset.validate();
    let scan = app.clone();
    let installed = blocking(move || scan.content.installed()).await;
    errors.extend(installed.check(&stored.preset));
    if !errors.is_empty() {
        return Err(mistakes("The host cannot run this preset yet.", errors));
    }

    if !options.force
        && let Some(players) = server::players(app.config.host.http_port).await
        && players > 0
    {
        let drivers = if players == 1 {
            "driver is"
        } else {
            "drivers are"
        };
        return Err(fail(
            StatusCode::CONFLICT,
            format!("{players} {drivers} on the server. Activating restarts it under them."),
        ));
    }

    // The running server read its config when it started, so it can be
    // rewritten underneath it.
    let files = render::render(&stored.preset, &app.config.host);
    let cfg = app.cfg_dir();
    blocking(move || files.write(&cfg)).await?;
    app.store().set_active(stored.clone())?;
    app.server
        .restart()
        .await
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    eprintln!(
        "{by} activated \"{}\" ({}, revision {}).",
        stored.preset.name, stored.id, stored.revision
    );
    Ok(Json(app.status().await))
}

async fn installed(State(app): State<Arc<App>>) -> Json<Installed> {
    Json(blocking(move || app.content.installed()).await)
}

async fn plan(
    State(app): State<Arc<App>>,
    body: Result<Json<Manifest>, JsonRejection>,
) -> Result<Json<UploadPlan>, Failure> {
    let Json(manifest) = body?;
    if let Some(file) = manifest
        .files
        .iter()
        .find(|file| !valid_sha256(&file.sha256))
    {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            format!(
                "The hash of {} is not a SHA-256 in lowercase hex.",
                file.path
            ),
        ));
    }
    Ok(Json(blocking(move || app.content.plan(&manifest)).await?))
}

async fn upload(
    State(app): State<Arc<App>>,
    Extension(User(by)): Extension<User>,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, Failure> {
    let file = classify(&path)
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "The server does not read that file."))?;
    let sha256 = headers
        .get(SHA256_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_sha256(value))
        .ok_or_else(|| {
            fail(
                StatusCode::BAD_REQUEST,
                format!("Send the file's SHA-256 in {SHA256_HEADER}, in lowercase hex."),
            )
        })?;
    let too_large = || {
        fail(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("Files go up to {} GiB.", MAX_FILE >> 30),
        )
    };
    let announced = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok()?.parse::<u64>().ok());
    if announced.is_some_and(|length| length > MAX_FILE) {
        return Err(too_large());
    }
    match app
        .content
        .upload(&file.path, sha256, body.into_data_stream())
        .await
    {
        Ok(()) => {
            eprintln!("{by} uploaded {}.", file.path);
            Ok(StatusCode::NO_CONTENT)
        }
        Err(UploadError::TooLarge) => Err(too_large()),
        Err(UploadError::Mismatch) => Err(fail(
            StatusCode::BAD_REQUEST,
            format!("The file does not match its {SHA256_HEADER}."),
        )),
        Err(UploadError::Body(error)) => Err(fail(
            StatusCode::BAD_REQUEST,
            format!("The upload broke off: {error}"),
        )),
        Err(UploadError::Io(error)) => Err(error.into()),
    }
}
