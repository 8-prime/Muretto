//! The host client against the real agent, with a shell standing in for
//! AssettoServer.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use muretto_core::{
    Install,
    host::{
        CONFLICT, Client, HostError, UNPROCESSABLE, local_files,
        protocol::{
            ServerState,
            presets::{AiMode, Entry, Preset, PresetId, TrackRef},
        },
        sync,
    },
    packages::Progress,
};
use muretto_host_agent::{
    App,
    config::{Config, Host, Tokens},
    router,
};

const TOKEN: &str = "0123456789abcdef";

/// Runs an agent serving from `dir` until the test ends; returns its URL.
fn agent(dir: &Path) -> String {
    let free_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = Config {
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
    };
    let (sender, address) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let app = App::open(config).unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            sender.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, router(app)).await.unwrap();
        });
    });
    format!("http://{}", address.recv().unwrap())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("muretto-host-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A game folder with a car, a second car that is only unpacked data, and a
/// track with two layouts.
fn install(dir: &Path) -> Install {
    let root = dir.join("game");
    for (path, contents) in [
        ("system/data/surfaces.ini", "[SURFACE_0]\n"),
        ("content/cars/alpha_gt3/data.acd", "alpha data"),
        ("content/cars/alpha_gt3/collider.kn5", "alpha collider"),
        ("content/cars/alpha_gt3/alpha_gt3.kn5", "the whole car"),
        ("content/cars/loose/data/car.ini", "[INFO]\n"),
        ("content/tracks/park/park.kn5", "park model"),
        ("content/tracks/park/models_long.ini", "[MODEL_0]\n"),
        ("content/tracks/park/models_short.ini", "[MODEL_0]\n"),
        (
            "content/tracks/park/long/data/surfaces.ini",
            "[SURFACE_0]\n",
        ),
        (
            "content/tracks/park/short/data/surfaces.ini",
            "[SURFACE_0]\n",
        ),
        ("content/tracks/park/long/ai/fast_lane.ai", "spline"),
        ("content/tracks/park/ui/long/ui_track.json", "{}"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    Install::at(root, None)
}

fn entry(model: &str) -> Entry {
    Entry {
        model: model.into(),
        skin: None,
        driver_name: None,
        team: None,
        guid: None,
        ballast_kg: 0,
        restrictor: 0,
        ai: AiMode::None,
    }
}

fn park(layout: &str) -> TrackRef {
    TrackRef {
        id: "park".into(),
        layout: Some(layout.into()),
    }
}

#[test]
fn the_files_of_the_preset_s_layout_are_picked_and_what_is_lacking_named() {
    let dir = scratch("files");
    let install = install(&dir);
    let preset = Preset::new(
        "Sprint",
        park("long"),
        vec![entry("alpha_gt3"), entry("loose"), entry("alpha_gt3")],
    );
    let (missing, files) = local_files(&install, &preset);
    assert_eq!(missing, ["loose"]);
    let mut paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        [
            "content/cars/alpha_gt3/collider.kn5",
            "content/cars/alpha_gt3/data.acd",
            "content/tracks/park/long/ai/fast_lane.ai",
            "content/tracks/park/long/data/surfaces.ini",
            "content/tracks/park/models_long.ini",
            "content/tracks/park/park.kn5",
            "system/data/surfaces.ini",
        ]
    );

    let elsewhere = Preset::new("Elsewhere", park("reverse"), vec![entry("alpha_gt3")]);
    assert_eq!(local_files(&install, &elsewhere).0, ["park (reverse)"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_preset_is_created_given_its_content_activated_and_kept_from_lost_updates() {
    let dir = scratch("api");
    let install = install(&dir);
    let url = agent(&dir.join("host"));
    let client = Client::new(&url, TOKEN);

    let status = client.status().unwrap();
    assert_eq!(status.server, ServerState::Idle);
    assert_eq!(status.active, None);

    let id = PresetId::new();
    let preset = Preset::new("Sprint", park("long"), vec![entry("alpha_gt3")]);
    let stored = client.create(id, &preset).unwrap();
    assert_eq!(stored.updated_by, "stefan");
    // A retried create finds the first.
    assert_eq!(
        client.create(id, &preset).unwrap_err().status(),
        Some(CONFLICT)
    );

    let list = client.presets().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].cars, ["alpha_gt3"]);
    assert!(!list[0].active);

    // The host has none of the content yet.
    let refused = client.activate(id, false).unwrap_err();
    assert_eq!(refused.status(), Some(UNPROCESSABLE));
    let fields: Vec<&str> = refused.fields().iter().map(|f| f.field.as_str()).collect();
    assert_eq!(fields, ["track", "entries[0].model"]);

    let progress = Progress::new();
    let synced = sync(&client, &install, &preset, &progress).unwrap();
    assert_eq!(synced.uploaded, 7);
    assert!(synced.missing.is_empty());
    assert_eq!(
        sync(&client, &install, &preset, &progress)
            .unwrap()
            .uploaded,
        0,
        "nothing twice"
    );
    let installed = client.installed().unwrap();
    assert_eq!(installed.cars[0].id, "alpha_gt3");
    assert!(installed.system_surfaces);

    let status = client.activate(id, false).unwrap();
    assert!(matches!(status.server, ServerState::Running { .. }));
    assert_eq!(status.active.as_ref().unwrap().name, "Sprint");
    assert!(client.presets().unwrap()[0].active);

    // Someone else saves first; the stale save is refused.
    let mut theirs = stored.preset.clone();
    theirs.name = "Sprint, longer".into();
    let saved = client.save(id, stored.revision, &theirs).unwrap();
    let mut mine = stored.preset.clone();
    mine.name = "Sprint, shorter".into();
    let stale = client.save(id, stored.revision, &mine).unwrap_err();
    assert_eq!(stale.status(), Some(CONFLICT));
    let fresh = client.preset(id).unwrap();
    assert_eq!(fresh.revision, saved.revision);
    assert!(client.status().unwrap().active.unwrap().changed_since);

    // Mistakes come back by field.
    let mut wrong = fresh.preset.clone();
    wrong.name = String::new();
    let refused = client.save(id, fresh.revision, &wrong).unwrap_err();
    assert_eq!(refused.fields()[0].field, "name");

    client.delete(id).unwrap();
    assert!(client.presets().unwrap().is_empty());

    let stranger = Client::new(&url, "not-the-token-at-all");
    assert_eq!(stranger.status().unwrap_err().status(), Some(401));
    let nowhere = Client::new("http://127.0.0.1:9", TOKEN);
    assert!(matches!(nowhere.status(), Err(HostError::Unreachable(_))));
    std::fs::remove_dir_all(&dir).unwrap();
}
