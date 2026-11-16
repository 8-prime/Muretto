//! The agent's API end to end, with a shell standing in for AssettoServer.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use muretto_host_agent::{
    App,
    config::{Config, Host, Tokens},
    router,
};
use muretto_host_protocol::{
    ApiError, ServerState, Status,
    content::{Installed, Manifest, ManifestFile, UploadPlan, classify},
    presets::{AiMode, Entry, Preset, PresetId, PresetSummary, StoredPreset, TrackRef},
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef";

/// A scratch directory, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("muretto-agent-api-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Agent {
    dir: PathBuf,
    app: Arc<App>,
    router: Router,
}

fn config(dir: &std::path::Path) -> Config {
    // Nothing listens on the HTTP port, so the agent never counts players.
    let free_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    Config {
        addr: "127.0.0.1:0".parse().unwrap(),
        server_dir: dir.to_owned(),
        state_dir: dir.join("muretto"),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 60".into()],
        host: Host {
            name: "Test box".into(),
            http_port: free_port,
            ..Host::default()
        },
        tokens: Tokens::parse(&format!("stefan:{TOKEN}")).unwrap(),
    }
}

fn host(dir: &Scratch) -> Agent {
    let app = App::open(config(&dir.0)).unwrap();
    Agent {
        dir: dir.0.clone(),
        router: router(app.clone()),
        app,
    }
}

struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&self.body)))
    }
}

impl Agent {
    async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        Reply { status, body }
    }

    fn request(method: Method, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(format!("/v1{path}"))
            .header("authorization", format!("Bearer {TOKEN}"))
    }

    async fn get(&self, path: &str) -> Reply {
        self.send(
            Self::request(Method::GET, path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn json(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &impl serde::Serialize,
    ) -> Reply {
        let mut request = Self::request(method, path).header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.send(
            request
                .body(Body::from(serde_json::to_vec(body).unwrap()))
                .unwrap(),
        )
        .await
    }

    async fn upload(&self, path: &str, bytes: &'static [u8], sha256: &str) -> Reply {
        self.send(
            Self::request(Method::PUT, &format!("/content/files/{path}"))
                .header("x-content-sha256", sha256)
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
    }
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn preset(name: &str) -> Preset {
    let entry = Entry {
        model: "ks_audi_r8_lms".into(),
        skin: None,
        driver_name: None,
        team: None,
        guid: None,
        ballast_kg: 0,
        restrictor: 0,
        ai: AiMode::None,
    };
    Preset::new(
        name,
        TrackRef {
            id: "ks_brands_hatch".into(),
            layout: Some("gp".into()),
        },
        vec![entry.clone(), entry],
    )
}

const CAR: &str = "content/cars/ks_audi_r8_lms/data.acd";
const TRACK: &str = "content/tracks/ks_brands_hatch/gp/data/surfaces.ini";

#[tokio::test]
async fn strangers_are_turned_away() {
    let dir = Scratch::new("strangers");
    let host = host(&dir);
    for auth in [None, Some("Bearer wrong-token-0000000"), Some(TOKEN)] {
        let mut request = Request::get("/v1/status");
        if let Some(auth) = auth {
            request = request.header("authorization", auth);
        }
        let reply = host.send(request.body(Body::empty()).unwrap()).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{auth:?}");
    }
}

#[tokio::test]
async fn presets_are_saved_against_the_revision_edited() {
    let dir = Scratch::new("revisions");
    let host = host(&dir);
    let id = PresetId::new();
    let path = format!("/presets/{id}");

    let reply = host.json(Method::PUT, &path, &[], &preset("Sunday")).await;
    assert_eq!(reply.status, StatusCode::PRECONDITION_REQUIRED);

    let reply = host
        .json(
            Method::PUT,
            &path,
            &[("if-none-match", "*")],
            &preset("Sunday"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: StoredPreset = reply.json();
    assert_eq!((stored.revision, stored.updated_by.as_str()), (1, "stefan"));

    // A retried create, and a save from someone who opened revision 0.
    for headers in [[("if-none-match", "*")], [("if-match", "0")]] {
        let reply = host
            .json(Method::PUT, &path, &headers, &preset("Sunday"))
            .await;
        assert_eq!(reply.status, StatusCode::CONFLICT);
    }

    let reply = host
        .json(
            Method::PUT,
            &path,
            &[("if-match", "\"1\"")],
            &preset("Sunday GT3"),
        )
        .await;
    assert_eq!(reply.json::<StoredPreset>().revision, 2);

    let list: Vec<PresetSummary> = host.get("/presets").await.json();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].name.as_str(), list[0].slots), ("Sunday GT3", 2));

    // What the agent knew survives a restart of it.
    let reply = self::host(&dir).get(&path).await;
    assert_eq!(reply.json::<StoredPreset>().revision, 2);
}

#[tokio::test]
async fn presets_with_mistakes_are_refused_field_by_field() {
    let dir = Scratch::new("mistakes");
    let host = host(&dir);
    let mut bad = preset("Bad");
    bad.name = "Bad\nADMIN_PASSWORD=x".into();
    bad.entries[1].model = "../x".into();
    let reply = host
        .json(
            Method::PUT,
            &format!("/presets/{}", PresetId::new()),
            &[("if-none-match", "*")],
            &bad,
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<_> = reply
        .json::<ApiError>()
        .fields
        .into_iter()
        .map(|f| f.field)
        .collect();
    assert_eq!(fields, ["name", "entries[1].model"]);
}

#[tokio::test]
async fn content_is_uploaded_once() {
    let dir = Scratch::new("content");
    let host = host(&dir);

    let reply = host
        .upload(
            "content/cars/ks_audi_r8_lms/ks_audi_r8_lms.kn5",
            b"x",
            &sha(b"x"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = host.upload(CAR, b"car", &sha(b"not the car")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);

    let manifest = Manifest {
        files: [(CAR, &b"car"[..]), (TRACK, b"surfaces")]
            .into_iter()
            .map(|(path, bytes)| ManifestFile {
                path: classify(path).unwrap().path,
                size: bytes.len() as u64,
                sha256: sha(bytes),
            })
            .collect(),
    };
    let plan: UploadPlan = host
        .json(Method::POST, "/content/plan", &[], &manifest)
        .await
        .json();
    assert_eq!(plan.upload.len(), 2);

    assert_eq!(
        host.upload(CAR, b"car", &sha(b"car")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        host.upload(TRACK, b"surfaces", &sha(b"surfaces"))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let plan: UploadPlan = host
        .json(Method::POST, "/content/plan", &[], &manifest)
        .await
        .json();
    assert_eq!(plan.upload, []);

    let installed: Installed = host.get("/content").await.json();
    assert_eq!(installed.cars[0].id, "ks_audi_r8_lms");
    assert_eq!(installed.tracks[0].layouts[0].id.as_deref(), Some("gp"));
}

#[tokio::test]
async fn activating_writes_the_config_and_starts_the_server() {
    let dir = Scratch::new("activate");
    let host = host(&dir);
    let id = PresetId::new();
    let path = format!("/presets/{id}");
    host.json(
        Method::PUT,
        &path,
        &[("if-none-match", "*")],
        &preset("Sunday"),
    )
    .await;

    let activate = format!("{path}/activate");
    let force = serde_json::json!({ "force": false });
    let reply = host.json(Method::POST, &activate, &[], &force).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<_> = reply
        .json::<ApiError>()
        .fields
        .into_iter()
        .map(|f| f.field)
        .collect();
    assert_eq!(fields, ["track", "entries[0].model", "entries[1].model"]);

    host.upload(CAR, b"car", &sha(b"car")).await;
    host.upload(TRACK, b"surfaces", &sha(b"surfaces")).await;
    let reply = host.json(Method::POST, &activate, &[], &force).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let status: Status = reply.json();
    assert!(matches!(status.server, ServerState::Running { .. }));
    let active = status.active.unwrap();
    assert_eq!(
        (active.id, active.revision, active.changed_since),
        (id, 1, false)
    );

    let cfg = std::fs::read_to_string(host.dir.join("cfg/server_cfg.ini")).unwrap();
    assert!(cfg.contains("NAME=Test box | Sunday\n"), "{cfg}");
    assert!(cfg.contains("CARS=ks_audi_r8_lms\n"));
    assert!(host.dir.join("cfg/entry_list.ini").exists());

    // Saving the active preset leaves the server as it is until reactivated;
    // deleting it does not stop the server either.
    host.json(
        Method::PUT,
        &path,
        &[("if-match", "1")],
        &preset("Sunday GT3"),
    )
    .await;
    let status: Status = host.get("/status").await.json();
    assert!(status.active.unwrap().changed_since);
    let reply = host.send(
        Agent::request(Method::DELETE, &path)
            .body(Body::empty())
            .unwrap(),
    );
    assert_eq!(reply.await.status, StatusCode::NO_CONTENT);
    let status: Status = host.get("/status").await.json();
    assert_eq!(status.active.unwrap().name, "Sunday");
    assert!(matches!(status.server, ServerState::Running { .. }));

    host.app.shutdown().await;
    let status: Status = host.get("/status").await.json();
    assert_eq!(status.server, ServerState::Idle);
}
