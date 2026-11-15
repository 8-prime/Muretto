//! Drives the real window with pointer and keyboard events against a small
//! game folder built on disk, and checks what the session becomes.

use std::{
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    prelude::*,
    px, size,
    test::{TestQueryExt as _, TestWindowExt as _},
};
use muretto::{
    game::{Game, Launch},
    shell::{Shell, Sources},
};
use muretto_core::{
    Error, Install, Library,
    group::{Code, Event, Group, Groups, Presence, Status, Viewed, relay::Memory},
    online::{
        self, Column, Facet, Found, Lobby, Server, Slot,
        offers::{Kind, Offer},
    },
    packages::{Progress, csp, recipes::PURE, weather_fx},
    session::SessionMode,
    steam::SteamUser,
};

/// A game folder with three cars, two tracks and one weather.
fn game_folder(name: &str) -> PathBuf {
    let root = std::env::temp_dir()
        .join(format!("muretto-ui-{name}-{}", std::process::id()))
        .join("steamapps/common/assettocorsa");
    let _ = std::fs::remove_dir_all(&root);
    let write = |path: &str, contents: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    for (id, name, tag) in [
        ("alpha_gt3", "Alpha GT3", "gt3"),
        ("beta_gt3", "Beta GT3", "gt3"),
        ("gamma_road", "Gamma Road", "street"),
    ] {
        write(
            &format!("content/cars/{id}/ui/ui_car.json"),
            &format!(
                r#"{{"name": "{name}", "brand": "Works", "class": "race", "tags": ["{tag}", "rwd"], "specs": {{"bhp": "500bhp"}}}}"#
            ),
        );
        write(&format!("content/cars/{id}/data/car.ini"), "");
        write(&format!("content/cars/{id}/skins/red/ui_skin.json"), "{}");
        write(&format!("content/cars/{id}/skins/blue/ui_skin.json"), "{}");
    }
    let track = |pit_boxes: u32, name: &str| {
        format!(
            r#"{{"name": "{name}", "country": "Italy", "length": "4000", "pitboxes": "{pit_boxes}", "run": "clockwise", "tags": ["circuit"]}}"#
        )
    };
    write(
        "content/tracks/park/ui/long/ui_track.json",
        &track(4, "Park - Long"),
    );
    write(
        "content/tracks/park/ui/short/ui_track.json",
        &track(4, "Park - Short"),
    );
    write("content/tracks/park/park.kn5", "");
    write("content/tracks/ring/ui/ui_track.json", &track(10, "Ring"));
    write("content/tracks/ring/ring.kn5", "");
    write(
        "content/weather/1_clear/weather.ini",
        "[LAUNCHER]\nNAME=Clear\nTEMPERATURE_COEFF=1\n",
    );
    write("cfg/video.ini", "[VIDEO]\n");
    root
}

/// Stands in for the patch's download site: lists two builds and hands out
/// a small archive for either, or fails like a dropped connection.
struct Shelf {
    online: bool,
}

impl csp::Source for Shelf {
    fn catalogue(&self) -> muretto_core::Result<csp::Catalogue> {
        if !self.online {
            return Err(Error::Network("No connection.".to_owned()));
        }
        Ok(csp::Catalogue::parse(
            r#"<li><a href="?info=0.2.11">v0.2.11</a> (119.12 MB, <span class="tag tag-recommended">recommended</span>)</li>
               <li><a href="?info=0.3.1">v0.3.1</a> (140.76 MB, <span class="tag tag-untested">untested</span>)</li>"#,
        ))
    }

    fn fetch(&self, release: &csp::Release, to: &Path, _: &Progress) -> muretto_core::Result<()> {
        if !self.online {
            return Err(Error::Network("Download failed: no connection.".to_owned()));
        }
        use std::io::Write as _;
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(to).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("dwrite.dll", options).unwrap();
        zip.write_all(b"loader").unwrap();
        zip.start_file("extension/config/data_manifest.ini", options)
            .unwrap();
        write!(
            zip,
            "[VERSION]\nSHADERS_PATCH={}\nSHADERS_PATCH_BUILD=1\n",
            release.version
        )
        .unwrap();
        zip.finish().unwrap();
        Ok(())
    }
}

/// Stands in for the official lobby: three servers, one on a track the
/// fixture lacks, and slots that leave only the second car free. Another
/// server is up but unlisted, and one more runs on the local network.
#[derive(Default)]
struct Paddock {
    /// Where the missing car can be downloaded straight away.
    car_url: String,
}

const PADDOCK: &str = r#"[
  {"ip":"10.0.0.1","port":9600,"cport":8081,"tport":9600,"name":"Park Sprint","clients":3,"maxclients":4,
   "track":"park-short","cars":["alpha_gt3","beta_gt3"],"session":3,"sessiontypes":[1,3],"durations":[10,8],
   "timeleft":300,"country":["Italy","IT"],"pass":false},
  {"ip":"10.0.0.2","port":9600,"cport":8081,"tport":9600,"name":"Ring Cruise ℹ8082","clients":1,"maxclients":10,
   "track":"ring","cars":["gamma_road","delta_missing","traffic_van"],"session":1,"sessiontypes":[1],"durations":[60],
   "timeleft":1200,"country":["Germany","DE"],"pass":false},
  {"ip":"10.0.0.3","port":9600,"cport":8081,"tport":9600,"name":"Faraway ℹ8082","clients":0,"maxclients":20,
   "track":"csp/2651/../faraway","cars":["alpha_gt3"],"session":1,"sessiontypes":[1],"durations":[60],
   "timeleft":0,"country":["Japan","JP"],"pass":true}
]"#;

impl online::Source for Paddock {
    fn player(&self) -> muretto_core::Result<SteamUser> {
        Ok(SteamUser {
            id: "76561198000000001".to_owned(),
            name: "L. Brandt".to_owned(),
        })
    }

    fn servers(&self, _: &SteamUser) -> muretto_core::Result<Lobby> {
        Ok(Lobby {
            servers: online::parse_lobby(PADDOCK)?,
            stale: None,
        })
    }

    fn slots(&self, server: &Server, _: &SteamUser) -> muretto_core::Result<Vec<Slot>> {
        let slot = |car: &str, driver: Option<&str>| Slot {
            car: car.to_owned(),
            skin: "blue".to_owned(),
            driver: driver.map(str::to_owned),
            connected: driver.is_some(),
            yours: false,
            ai: false,
        };
        Ok(match server.name.as_str() {
            "Park Sprint" => vec![
                slot("alpha_gt3", Some("T. Okafor")),
                slot("alpha_gt3", Some("M. Lindqvist")),
                slot("beta_gt3", Some("R. Castellano")),
                slot("beta_gt3", None),
            ],
            "Ring Cruise" => vec![
                slot("gamma_road", Some("J. van Dijk")),
                slot("gamma_road", None),
                slot("delta_missing", None),
                Slot {
                    ai: true,
                    ..slot("traffic_van", None)
                },
            ],
            _ => Vec::new(),
        })
    }

    fn info(&self, ip: &str, http_port: u16) -> muretto_core::Result<Server> {
        if ip != "10.0.0.9" {
            return Err(Error::Network("No answer.".to_owned()));
        }
        online::parse_info(
            r#"{"port":9600,"tport":9600,"name":"Garage","clients":2,"maxclients":8,"track":"ring",
                "cars":["gamma_road"],"session":1,"sessiontypes":[1]}"#,
            ip,
            http_port,
        )
    }

    fn lan(&self) -> muretto_core::Result<Vec<Server>> {
        let mut server = online::parse_info(
            r#"{"port":9600,"tport":9600,"name":"Office LAN","clients":0,"maxclients":8,"track":"park-long",
                "cars":["alpha_gt3"],"session":1,"sessiontypes":[1]}"#,
            "192.168.1.20",
            8081,
        )?;
        server.found = Found::Lan;
        Ok(vec![server])
    }

    fn ping(&self, targets: &[SocketAddr], answered: &mut dyn FnMut(SocketAddr, Duration)) {
        for target in targets {
            match target.ip().to_string().as_str() {
                "10.0.0.1" => answered(*target, Duration::from_millis(30)),
                "10.0.0.2" => answered(*target, Duration::from_millis(120)),
                _ => {}
            }
        }
    }

    fn offers(&self, server: &Server) -> muretto_core::Result<Vec<Offer>> {
        Ok(match server.name.as_str() {
            "Ring Cruise" => vec![Offer {
                kind: Kind::Car,
                id: "delta_missing".to_owned(),
                version: Some("1.2".to_owned()),
                url: self.car_url.clone(),
            }],
            "Faraway" => vec![Offer {
                kind: Kind::Track,
                id: String::new(),
                version: None,
                url: "https://drive.example.com/file/faraway".to_owned(),
            }],
            _ => Vec::new(),
        })
    }
}

/// Serves `body` once over HTTP on the loopback interface; returns its URL.
fn serve_once(name: &str, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/{name}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });
    url
}

/// A zip holding the files given as (path, contents).
fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, contents) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn open(cx: &mut TestAppContext, root: &Path) -> (AnyWindowHandle, Entity<Game>, Entity<Shell>) {
    open_with(cx, root, Shelf { online: true }, Paddock::default())
}

fn open_with(
    cx: &mut TestAppContext,
    root: &Path,
    patches: Shelf,
    servers: Paddock,
) -> (AnyWindowHandle, Entity<Game>, Entity<Shell>) {
    open_with_groups(cx, root, patches, servers, Memory::default())
}

/// Opens Muretto with its groups meeting on `groups`, a group server in
/// memory that a test can also join as a friend.
fn open_with_groups(
    cx: &mut TestAppContext,
    root: &Path,
    patches: Shelf,
    servers: Paddock,
    groups: Memory,
) -> (AnyWindowHandle, Entity<Game>, Entity<Shell>) {
    let install = Install::at(root.to_path_buf(), None);
    let library = Library::scan(&install);
    assert_eq!((library.cars.len(), library.tracks.len()), (3, 2));
    cx.update(|cx| {
        muretto::init(cx);
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1440.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let sources = Sources {
                    patches: Arc::new(patches),
                    servers: Arc::new(servers),
                    // Beside the game folder, so each test keeps its own.
                    saved_servers: Some(saved_servers(root)),
                    saved_groups: Some(saved_groups(root)),
                    group_relay: Some(Arc::new(groups)),
                };
                cx.new(|cx| Shell::with_content(install, library, sources, window, cx))
            },
        )
        .unwrap();
        let game = shell
            .read(cx)
            .game()
            .expect("the fixture is drivable")
            .clone();
        (window, game, shell)
    })
}

fn saved_servers(root: &Path) -> PathBuf {
    root.parent().unwrap().join("servers.json")
}

fn saved_groups(root: &Path) -> PathBuf {
    root.parent().unwrap().join("groups.json")
}

/// One user action, followed by everything it sets in motion. Events a view
/// emits are delivered when the update ends, so each step is its own update.
fn act(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    step: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        step(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_first_session_fits_the_first_track(cx: &mut TestAppContext) {
    let root = game_folder("first");
    let (_, game, _) = open(cx, &root);
    cx.update(|cx| {
        let game = game.read(cx);
        let session = game.session();
        assert_eq!(session.car_id, "alpha_gt3");
        assert_eq!(session.track_id, "park");
        assert_eq!(session.layout_id.as_deref(), Some("long"));
        // Park has four pit boxes: three opponents at most.
        assert_eq!(session.opponents, 3);
        assert_eq!(session.summary(&game.library), "Race · 12 laps · 4 cars");
    });
}

#[gpui_kit::test]
fn clicking_a_mode_tile_changes_the_session(cx: &mut TestAppContext) {
    let root = game_folder("mode");
    let (window, game, _) = open(cx, &root);
    act(cx, window, |window, cx| {
        window.click(("mode", SessionMode::Hotlap as usize), cx)
    });
    cx.update(|cx| {
        let game = game.read(cx);
        assert_eq!(game.session().mode, SessionMode::Hotlap);
        assert_eq!(game.session().summary(&game.library), "Hotlap");
    });
}

#[gpui_kit::test]
fn a_car_is_found_by_typing_and_picked_by_clicking(cx: &mut TestAppContext) {
    let root = game_folder("car");
    let (window, game, shell) = open(cx, &root);
    act(cx, window, |window, cx| window.click("change-car", cx));
    cx.update(|cx| assert!(shell.read(cx).is_choosing()));
    // The search box takes focus when the list opens.
    act(cx, window, |window, cx| window.input("beta", cx));
    act(cx, window, |window, cx| window.click(0usize, cx));
    cx.update(|cx| {
        assert!(!shell.read(cx).is_choosing());
        let session = game.read(cx).session();
        assert_eq!(session.car_id, "beta_gt3");
        // The livery belongs to the new car, not the old one.
        assert_eq!(session.skin_id, "blue");
    });
}

#[gpui_kit::test]
fn a_new_track_resets_layout_and_field_size(cx: &mut TestAppContext) {
    let root = game_folder("track");
    let (window, game, _) = open(cx, &root);
    cx.update(|cx| {
        game.update(cx, |game, cx| {
            game.edit(cx, |s| s.layout_id = Some("short".into()))
        })
    });
    act(cx, window, |window, cx| window.click("change-track", cx));
    // Rows are in name order: Park, Ring.
    act(cx, window, |window, cx| window.click(1usize, cx));
    cx.update(|cx| {
        let session = game.read(cx).session();
        assert_eq!(session.track_id, "ring");
        assert_eq!(session.layout_id, None);
        assert_eq!(session.opponents, 3);
    });
}

#[gpui_kit::test]
fn escape_closes_the_list_without_choosing(cx: &mut TestAppContext) {
    let root = game_folder("escape");
    let (window, game, shell) = open(cx, &root);
    act(cx, window, |window, cx| window.click("change-car", cx));
    cx.update(|cx| assert!(shell.read(cx).is_choosing()));
    act(cx, window, |window, cx| window.press("escape", cx));
    cx.update(|cx| {
        assert!(!shell.read(cx).is_choosing());
        assert_eq!(game.read(cx).session().car_id, "alpha_gt3");
    });
}

#[gpui_kit::test]
fn drive_writes_the_session_and_explains_a_failed_start(cx: &mut TestAppContext) {
    let root = game_folder("drive");
    let (window, game, _) = open(cx, &root);
    act(cx, window, |window, cx| window.click("drive", cx));
    cx.update(|cx| {
        let game = game.read(cx);
        // The fixture has no simulator executable, so nothing can start.
        assert!(matches!(&game.launch, Launch::Failed(reason) if reason.contains("acs.exe")));
        let cfg = game.install.cfg_dir();
        let race = std::fs::read_to_string(cfg.join("race.ini")).unwrap();
        assert!(race.contains("TRACK=park\r\n"));
        assert!(race.contains("CONFIG_TRACK=long\r\n"));
        assert!(race.contains("CARS=4\r\n"));
        assert!(cfg.join("assists.ini").is_file());
        // Defaults the game needs were copied in beside it.
        assert!(cfg.join("video.ini").is_file());
    });
}

#[gpui_kit::test]
fn typing_more_opponents_than_pit_boxes_is_corrected(cx: &mut TestAppContext) {
    let root = game_folder("opponents");
    let (window, game, _) = open(cx, &root);
    act(cx, window, |window, cx| window.click("opponents", cx));
    act(cx, window, |window, cx| window.press("backspace", cx));
    act(cx, window, |window, cx| window.input("9", cx));
    cx.update(|cx| assert_eq!(game.read(cx).session().opponents, 3));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("opponents").value(), Some("3"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_a_slider_to_its_end_sets_the_limit(cx: &mut TestAppContext) {
    let root = game_folder("slider");
    let (window, game, _) = open(cx, &root);
    cx.update(|cx| assert_eq!(game.read(cx).session().ai_strength, 96));
    act(cx, window, |window, cx| {
        let rail = window.find_by_label("AI strength").bounds();
        let handle = gpui_kit::point(rail.left() + rail.size.width * 0.86, rail.center().y);
        window.drag(
            handle,
            gpui_kit::point(rail.left() - px(40.), rail.center().y),
            cx,
        );
    });
    cx.update(|cx| assert_eq!(game.read(cx).session().ai_strength, 70));
}

const SETTINGS: (&str, usize) = ("section", 4);
const PATCH_PAGE: (&str, usize) = ("Custom Shaders Patch", 0);

#[gpui_kit::test]
fn the_patch_is_installed_updated_and_removed_from_settings(cx: &mut TestAppContext) {
    let root = game_folder("patch");
    let (window, game, _) = open(cx, &root);
    cx.update(|cx| assert_eq!(game.read(cx).patch, None));

    // The main button offers the recommended build, not the newest.
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(PATCH_PAGE, cx));
    act(cx, window, |window, cx| window.click("patch-install", cx));
    cx.update(|cx| {
        let patch = game.read(cx).patch.clone().expect("the patch is installed");
        assert_eq!(patch.version.as_deref(), Some("0.2.11"));
        assert!(patch.managed);
    });
    assert_eq!(std::fs::read(root.join("dwrite.dll")).unwrap(), b"loader");
    // Up to date now, so the offer is gone.
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("patch-install").is_none());
    })
    .unwrap();

    // Any listed build can be installed over it; the newest is first.
    act(cx, window, |window, cx| {
        window.click(("patch-version", 0usize), cx)
    });
    cx.update(|cx| {
        assert_eq!(
            game.read(cx).patch.as_ref().unwrap().version.as_deref(),
            Some("0.3.1")
        );
    });

    // Removal asks first.
    act(cx, window, |window, cx| window.click("patch-remove", cx));
    assert!(root.join("dwrite.dll").exists());
    act(cx, window, |window, cx| {
        window.click("patch-remove-confirm", cx)
    });
    cx.update(|cx| assert_eq!(game.read(cx).patch, None));
    assert!(!root.join("dwrite.dll").exists() && !root.join("extension").exists());
}

#[gpui_kit::test]
fn without_a_connection_the_known_builds_are_offered_and_failure_is_shown(cx: &mut TestAppContext) {
    let root = game_folder("offline");
    let (window, game, _) = open_with(cx, &root, Shelf { online: false }, Paddock::default());
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(PATCH_PAGE, cx));
    act(cx, window, |window, cx| window.click("patch-install", cx));
    cx.update(|cx| assert_eq!(game.read(cx).patch, None));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        // The failure stays on screen until dismissed, and nothing was written.
        assert!(window.try_find("patch-dismiss").is_some());
    })
    .unwrap();
    assert!(!root.join("dwrite.dll").exists());
    assert!(!root.join(".muretto/downloads").exists());
}

const WEATHER_PAGE: (&str, usize) = ("Custom Shaders Patch", 3);

/// Puts a copy of the patch into the game folder, as if installed earlier.
fn with_patch(root: &Path) {
    let write = |path: &str, contents: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    write("dwrite.dll", "loader");
    write(
        "extension/config/data_manifest.ini",
        "[VERSION]\nSHADERS_PATCH=0.2.11\n",
    );
    write(
        "extension/config/weather_fx.ini",
        "[BASIC]\nENABLED=1\nCONTROLLER=sol\nIMPLEMENTATION=base\n",
    );
    write("extension/weather/base/weather.lua", "");
    write(
        "extension/weather/base/manifest.ini",
        "[ABOUT]\nNAME = Default\nVERSION = 1.6\n",
    );
}

/// A zip laid out the way Pure's is, wrapped in a folder.
fn pure_archive(root: &Path) -> PathBuf {
    use std::io::Write as _;
    let path = root.parent().unwrap().join("Pure 0.257.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, contents) in [
        ("Pure 0.257/extension/weather/pure/weather.lua", "script"),
        (
            "Pure 0.257/extension/weather/pure/manifest.ini",
            "[ABOUT]\nNAME = Pure\nAUTHOR = Peter Boese\nVERSION = 0.257\n",
        ),
        (
            "Pure 0.257/extension/weather-controllers/pureCtrl/controller.lua",
            "ctrl",
        ),
        ("Pure 0.257/apps/lua/Pure Config/manifest.ini", "app"),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

#[gpui_kit::test]
fn pure_is_got_through_the_dialog_put_to_use_and_removed(cx: &mut TestAppContext) {
    let root = game_folder("pure");
    with_patch(&root);
    let archive = pure_archive(&root);
    let (window, game, shell) = open(cx, &root);
    let install = cx.update(|cx| game.read(cx).install.clone());

    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(WEATHER_PAGE, cx));
    act(cx, window, |window, cx| {
        window.click(("recipe-get", 0usize), cx)
    });
    // Its page opens in the browser while the dialog waits for the archive.
    assert_eq!(cx.opened_url().as_deref(), Some(PURE.page));
    let dialog = cx.update(|cx| {
        shell
            .read(cx)
            .getting()
            .expect("the dialog is open")
            .clone()
    });

    // What a drop or the file chooser hands over.
    cx.update(|cx| dialog.update(cx, |dialog, cx| dialog.accept(archive, cx)));
    cx.run_until_parked();
    let installed = PURE.installed(&install).expect("Pure is installed");
    assert_eq!(installed.version.as_deref(), Some("0.257"));
    assert!(!PURE.in_use(&install));

    act(cx, window, |window, cx| window.click("get-use", cx));
    assert!(PURE.in_use(&install));
    assert_eq!(weather_fx::settings(&install).controller, "pureCtrl");
    cx.update(|cx| assert!(shell.read(cx).getting().is_none()));

    // On the page it is now a style like any other, and can be removed.
    // Styles are in name order: Default, Pure.
    act(cx, window, |window, cx| {
        window.click(("style-remove", 1usize), cx)
    });
    assert!(PURE.installed(&install).is_some(), "removal asks first");
    act(cx, window, |window, cx| {
        window.click(("style-remove-confirm", 1usize), cx)
    });
    assert_eq!(PURE.installed(&install), None);
    assert_eq!(weather_fx::settings(&install).style, "base");
    assert!(root.join("extension/weather/base/weather.lua").is_file());
}

#[gpui_kit::test]
fn the_wrong_archive_is_refused_and_the_dialog_keeps_waiting(cx: &mut TestAppContext) {
    let root = game_folder("pure-wrong");
    with_patch(&root);
    let (window, game, shell) = open(cx, &root);
    let install = cx.update(|cx| game.read(cx).install.clone());
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(WEATHER_PAGE, cx));
    act(cx, window, |window, cx| {
        window.click(("recipe-get", 0usize), cx)
    });
    let dialog = cx.update(|cx| shell.read(cx).getting().unwrap().clone());

    let not_a_zip = root.parent().unwrap().join("notes.txt");
    std::fs::write(&not_a_zip, "just text").unwrap();
    cx.update(|cx| dialog.update(cx, |dialog, cx| dialog.accept(not_a_zip, cx)));
    cx.run_until_parked();

    assert_eq!(PURE.installed(&install), None);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        // Still open, still offering the drop zone.
        assert!(window.try_find("get-choose-file").is_some());
    })
    .unwrap();
    act(cx, window, |window, cx| window.press("escape", cx));
    cx.update(|cx| assert!(shell.read(cx).getting().is_none()));
}

#[gpui_kit::test]
fn pure_waits_for_the_patch(cx: &mut TestAppContext) {
    let root = game_folder("pure-no-patch");
    let (window, _, _) = open(cx, &root);
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(WEATHER_PAGE, cx));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("recipe-get", 0usize)).is_none());
    })
    .unwrap();
    // The way forward leads to the patch's page.
    act(cx, window, |window, cx| {
        window.click(("recipe-needs-patch", 0usize), cx)
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("patch-install").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn servers_are_narrowed_by_ticking_a_track_and_joined(cx: &mut TestAppContext) {
    let root = game_folder("online");
    let (window, game, shell) = open(cx, &root);
    act(cx, window, |window, cx| {
        window.click(("section", 2usize), cx)
    });
    let online = cx.update(|cx| shell.read(cx).online().unwrap().clone());
    let names = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let online = online.read(cx);
            online
                .matching()
                .iter()
                .map(|&i| online.listing().servers[i].server.name.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(names(cx), ["Park Sprint", "Ring Cruise", "Faraway"]);

    // Tick a track from the list of tracks servers run, by typing its name
    // and pressing enter.
    act(cx, window, |window, cx| window.click(("facet", 0usize), cx));
    cx.update(|cx| assert!(online.read(cx).picker().is_some()));
    act(cx, window, |window, cx| window.input("ring", cx));
    act(cx, window, |window, cx| window.press("enter", cx));
    act(cx, window, |window, cx| window.press("escape", cx));
    cx.update(|cx| {
        let online = online.read(cx);
        assert!(online.picker().is_none());
        assert!(online.filter().tracks.contains("ring"));
    });
    assert_eq!(names(cx), ["Ring Cruise"]);

    // The chip for the choice takes it away again.
    act(cx, window, |window, cx| {
        window.click(("chosen", 0usize), cx)
    });
    assert_eq!(names(cx).len(), 3);

    // Choosing a server asks it for its slots; the car with room is offered.
    act(cx, window, |window, cx| {
        window.click(("server", 0usize), cx)
    });
    cx.update(|cx| {
        let online = online.read(cx);
        assert_eq!(online.selected().unwrap().server.name, "Park Sprint");
        assert_eq!(online.chosen_car(), Some("beta_gt3"));
    });
    act(cx, window, |window, cx| window.click("only-track", cx));
    assert_eq!(names(cx), ["Park Sprint"]);
    cx.update(|cx| {
        assert_eq!(
            online.read(cx).filter().chosen(Facet::Track),
            ["park".to_owned()]
        )
    });

    act(cx, window, |window, cx| window.click("join", cx));
    cx.update(|cx| {
        let summary = online.read(cx).summary(cx).unwrap();
        // The fixture has no simulator executable, so nothing can start.
        assert!(matches!(&summary.launch, Launch::Failed(reason) if reason.contains("acs.exe")));
        let race =
            std::fs::read_to_string(game.read(cx).install.cfg_dir().join("race.ini")).unwrap();
        for line in [
            "ACTIVE=1",
            "SERVER_IP=10.0.0.1",
            "REQUESTED_CAR=beta_gt3",
            "TRACK=park",
            "CONFIG_TRACK=short",
            "SKIN=blue",
            "GUID=76561198000000001",
        ] {
            assert!(race.contains(&format!("{line}\r\n")), "{line} in {race}");
        }
    });
}

/// Lets work on other threads (pings) land, then whatever it set in motion.
fn settle(cx: &mut TestAppContext) {
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn servers_are_sorted_starred_found_on_the_network_and_completed(cx: &mut TestAppContext) {
    let root = game_folder("online-more");
    // Kept from an earlier run: a favourite the lobby does not list, one
    // that is gone, and a recently joined server.
    std::fs::write(
        saved_servers(&root),
        r#"{"favourites":[{"address":"10.0.0.9:8081","name":"Garage","at":1},
                          {"address":"10.0.0.8:8081","name":"Gone","at":2}],
            "recent":[{"address":"10.0.0.1:8081","name":"Park Sprint","at":3}]}"#,
    )
    .unwrap();
    let car = zip_bytes(&[
        (
            "Delta 1.2/content/cars/delta_missing/ui/ui_car.json",
            r#"{"name": "Delta Missing", "brand": "Works", "class": "street"}"#,
        ),
        ("Delta 1.2/content/cars/delta_missing/data/car.ini", ""),
        (
            "Delta 1.2/content/cars/delta_missing/skins/red/ui_skin.json",
            "{}",
        ),
    ]);
    let paddock = Paddock {
        car_url: serve_once("delta_missing.zip", car),
    };
    let (window, game, shell) = open_with(cx, &root, Shelf { online: true }, paddock);
    act(cx, window, |window, cx| {
        window.click(("section", 2usize), cx)
    });
    settle(cx);
    let online = cx.update(|cx| shell.read(cx).online().unwrap().clone());
    let names = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let online = online.read(cx);
            online
                .matching()
                .iter()
                .map(|&i| online.listing().servers[i].server.name.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        names(cx),
        ["Park Sprint", "Ring Cruise", "Faraway"],
        "busiest first; the unlisted favourite is not in the lobby's list"
    );

    // Pings arrived; sorting by them puts the silent server last either way.
    act(cx, window, |window, cx| {
        window.click(("sort", Column::Ping as usize), cx)
    });
    assert_eq!(names(cx), ["Park Sprint", "Ring Cruise", "Faraway"]);
    act(cx, window, |window, cx| {
        window.click(("sort", Column::Ping as usize), cx)
    });
    assert_eq!(names(cx), ["Ring Cruise", "Park Sprint", "Faraway"]);
    act(cx, window, |window, cx| {
        window.click(("sort", Column::Name as usize), cx)
    });
    assert_eq!(names(cx), ["Faraway", "Park Sprint", "Ring Cruise"]);

    // Starring Ring Cruise puts it in Favourites, beside the favourite that
    // answered when asked directly.
    act(cx, window, |window, cx| window.click(("star", 2usize), cx));
    act(cx, window, |window, cx| window.click(("tab", 1usize), cx));
    assert_eq!(names(cx), ["Garage", "Ring Cruise"]);
    cx.update(|cx| {
        let online = online.read(cx);
        assert_eq!(online.saved().favourites.len(), 3);
        let garage = online
            .listing()
            .servers
            .iter()
            .find(|l| l.server.name == "Garage")
            .unwrap();
        assert_eq!(garage.server.found, Found::Direct);
    });
    act(cx, window, |window, cx| window.click(("tab", 2usize), cx));
    assert_eq!(names(cx), ["Park Sprint"]);
    act(cx, window, |window, cx| window.click(("tab", 3usize), cx));
    settle(cx);
    assert_eq!(names(cx), ["Office LAN"]);

    // Ring Cruise: the traffic van is not a car to choose, and the missing
    // car downloads straight from the link the server gives.
    act(cx, window, |window, cx| window.click(("tab", 0usize), cx));
    act(cx, window, |window, cx| {
        window.click(("server", 2usize), cx)
    });
    settle(cx);
    cx.update(|cx| {
        let online = online.read(cx);
        assert_eq!(online.selected().unwrap().server.name, "Ring Cruise");
        assert!(online.is_traffic("traffic_van"));
        assert!(!online.is_traffic("delta_missing"));
        assert_eq!(online.chosen_car(), Some("gamma_road"));
        assert_eq!(online.car_info(), None, "car details start closed");
    });
    // The info button opens the car's details under its row without
    // choosing it, and closes them again.
    act(cx, window, |window, cx| {
        window.click(("car-info", 0usize), cx)
    });
    cx.update(|cx| {
        let online = online.read(cx);
        assert_eq!(online.car_info(), Some("gamma_road"));
        assert_eq!(online.chosen_car(), Some("gamma_road"));
    });
    act(cx, window, |window, cx| {
        window.click(("car-info", 0usize), cx)
    });
    cx.update(|cx| assert_eq!(online.read(cx).car_info(), None));
    act(cx, window, |window, cx| window.click(("fetch", 0usize), cx));
    settle(cx);
    cx.update(|cx| {
        assert!(game.read(cx).library.car("delta_missing").is_some());
        let online = online.read(cx);
        assert!(!online.is_fetching());
        assert!(online.listing().car("delta_missing").unwrap().installed);
    });
    act(cx, window, |window, cx| window.click(("car", 1usize), cx));
    cx.update(|cx| assert_eq!(online.read(cx).chosen_car(), Some("delta_missing")));

    // Faraway's track is on a web page: a dialog takes what was downloaded.
    act(cx, window, |window, cx| {
        window.click(("server", 0usize), cx)
    });
    settle(cx);
    act(cx, window, |window, cx| window.click(("page", 0usize), cx));
    let dialog = cx.update(|cx| {
        online
            .read(cx)
            .dialog()
            .expect("the dialog is open")
            .clone()
    });
    let unpacked = root.parent().unwrap().join("Downloads/faraway");
    std::fs::create_dir_all(unpacked.join("ui")).unwrap();
    std::fs::write(
        unpacked.join("ui/ui_track.json"),
        r#"{"name": "Faraway", "pitboxes": "8"}"#,
    )
    .unwrap();
    std::fs::write(unpacked.join("faraway.kn5"), "").unwrap();
    cx.update(|cx| dialog.update(cx, |dialog, cx| dialog.accept(unpacked.clone(), cx)));
    settle(cx);
    act(cx, window, |window, cx| window.click("content-done", cx));
    cx.update(|cx| {
        let online = online.read(cx);
        assert!(online.dialog().is_none());
        assert!(online.selected().unwrap().track_installed);
    });
}

/// Lets threads and what they set in motion run until `done` holds.
fn settle_until(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !done(cx) {
        assert!(
            std::time::Instant::now() < until,
            "timed out waiting for {what}"
        );
        settle(cx);
    }
}

#[gpui_kit::test]
fn friends_in_a_group_open_and_follow_each_other_s_server(cx: &mut TestAppContext) {
    let root = game_folder("friends");
    let _ = std::fs::remove_file(saved_groups(&root));
    let memory = Memory::default();
    let (window, _, shell) = open_with_groups(
        cx,
        &root,
        Shelf { online: true },
        Paddock::default(),
        memory.clone(),
    );
    act(cx, window, |window, cx| {
        window.click(("section", 2usize), cx)
    });
    settle(cx);
    let online = cx.update(|cx| shell.read(cx).online().unwrap().clone());
    act(cx, window, |window, cx| window.click(("tab", 4usize), cx));
    act(cx, window, |window, cx| window.click("group-start", cx));
    let code = cx.update(|cx| online.read(cx).friends().codes())[0].clone();
    settle_until(cx, "the group to connect", |cx| {
        cx.update(|cx| online.read(cx).friends().status(&code) == Some(&Status::Connected))
    });

    // A friend joins with the code and has Ring Cruise open.
    let seen = Arc::new(std::sync::Mutex::new(Vec::<Presence>::new()));
    let alex = Group::join(
        Arc::new(memory.clone()),
        Code::parse(&code).unwrap(),
        Presence {
            device: "alex".into(),
            name: "Alex".into(),
            viewing: Some(Viewed {
                ip: "10.0.0.2".into(),
                http_port: 8081,
            }),
        },
        {
            let seen = seen.clone();
            move |event| {
                if let Event::Members(members) = event {
                    *seen.lock().unwrap() = members.into_iter().map(|m| m.presence).collect();
                }
            }
        },
    );
    settle_until(cx, "the friend", |cx| {
        cx.update(|cx| online.read(cx).friends().online() == 1)
    });
    // The friend sees this player, with nothing open yet.
    let player = |seen: &[Presence]| seen.first().map(|p| (p.name.clone(), p.viewing.clone()));
    assert_eq!(
        player(&seen.lock().unwrap()),
        Some(("L. Brandt".into(), None))
    );

    // Opening the friend's server shows it, and the friend learns of it.
    act(cx, window, |window, cx| {
        window.click(("friend-jump", 0usize), cx)
    });
    cx.update(|cx| {
        assert_eq!(
            online.read(cx).selected().unwrap().server.name,
            "Ring Cruise"
        )
    });
    let ring = Some(Viewed {
        ip: "10.0.0.2".into(),
        http_port: 8081,
    });
    settle_until(cx, "the friend to see Ring Cruise", |_| {
        player(&seen.lock().unwrap()) == Some(("L. Brandt".into(), ring.clone()))
    });

    // Following: the friend moves to a server the lobby does not list; it is
    // asked directly and opened.
    act(cx, window, |window, cx| {
        window.click(("friend-follow", 0usize), cx)
    });
    cx.update(|cx| assert_eq!(online.read(cx).friends().following(), Some("alex")));
    alex.tell(Presence {
        device: "alex".into(),
        name: "Alex".into(),
        viewing: Some(Viewed {
            ip: "10.0.0.9".into(),
            http_port: 8081,
        }),
    });
    settle_until(cx, "the followed server", |cx| {
        cx.update(|cx| {
            online
                .read(cx)
                .selected()
                .is_some_and(|listed| listed.server.name == "Garage")
        })
    });
    cx.update(|cx| {
        assert_eq!(
            online.read(cx).selected().unwrap().server.found,
            Found::Direct
        )
    });

    // Picking a server by hand ends following.
    act(cx, window, |window, cx| window.click(("tab", 0usize), cx));
    act(cx, window, |window, cx| {
        window.click(("server", 0usize), cx)
    });
    cx.update(|cx| assert_eq!(online.read(cx).friends().following(), None));

    // The group is kept: Muretto opened again rejoins it by itself.
    assert_eq!(Groups::load(Some(saved_groups(&root))).groups.len(), 1);
    assert_eq!(memory.members(), 2);
    let (_, _, again) = open_with_groups(
        cx,
        &root,
        Shelf { online: true },
        Paddock::default(),
        memory.clone(),
    );
    settle_until(cx, "the second window to rejoin", |_| memory.members() == 3);
    cx.update(|cx| {
        let online = again.read(cx).online().unwrap().read(cx);
        assert_eq!(online.friends().codes(), std::slice::from_ref(&code));
    });

    // A code that is not one is refused; one already kept changes nothing.
    cx.update(|cx| {
        online.update(cx, |online, cx| {
            assert!(!online.join_code("not a code", cx));
            assert!(online.join_code(&code.to_lowercase(), cx));
            assert_eq!(online.friends().codes().len(), 1);
        })
    });

    // Leaving asks once, then forgets the code.
    act(cx, window, |window, cx| window.click(("tab", 4usize), cx));
    act(cx, window, |window, cx| {
        window.click(("group-leave", 0usize), cx)
    });
    cx.update(|cx| assert_eq!(online.read(cx).friends().codes().len(), 1));
    act(cx, window, |window, cx| {
        window.click(("group-leave", 0usize), cx)
    });
    cx.update(|cx| assert!(online.read(cx).friends().codes().is_empty()));
    assert!(Groups::load(Some(saved_groups(&root))).groups.is_empty());
    settle_until(cx, "the first window to leave the room", |_| {
        memory.members() == 2
    });
    drop(alex);
}

const CONTROLS_PAGE: (&str, usize) = ("Assetto Corsa", 2);
const ASSISTS_PAGE: (&str, usize) = ("Assetto Corsa", 3);

fn cfg_file(root: &Path, name: &str) -> String {
    let path = Install::at(root.to_path_buf(), None).cfg_dir().join(name);
    std::fs::read_to_string(path).unwrap_or_default()
}

#[gpui_kit::test]
fn a_video_setting_is_written_at_once_and_restored(cx: &mut TestAppContext) {
    let root = game_folder("video-settings");
    std::fs::write(root.join("cfg/audio.ini"), "[LEVELS]\r\nMASTER=1\r\n").unwrap();
    let (window, _, _) = open(cx, &root);
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    // Video is the first page.
    act(cx, window, |window, cx| {
        window.click("setting-vertical-sync", cx)
    });
    assert_eq!(cfg_file(&root, "video.ini"), "[VIDEO]\r\nVSYNC=1\r\n");
    // The game's other defaults come along so it still starts.
    assert!(cfg_file(&root, "audio.ini").contains("MASTER=1"));

    act(cx, window, |window, cx| window.click("restore-page", cx));
    assert!(
        cfg_file(&root, "video.ini").contains("VSYNC=1"),
        "restoring asks first"
    );
    act(cx, window, |window, cx| {
        window.click("restore-page-confirm", cx)
    });
    assert_eq!(cfg_file(&root, "video.ini"), "[VIDEO]\r\n");
}

#[gpui_kit::test]
fn assists_set_in_settings_reach_the_session_and_carry_over(cx: &mut TestAppContext) {
    use muretto_core::session::AssistPreset;
    let root = game_folder("assists-settings");
    let (window, game, _) = open(cx, &root);
    cx.update(|cx| {
        assert_eq!(
            game.read(cx).session().assists.preset(),
            Some(AssistPreset::Pro)
        )
    });
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(ASSISTS_PAGE, cx));
    act(cx, window, |window, cx| {
        window.click("setting-ideal-line", cx)
    });
    cx.update(|cx| {
        let assists = &game.read(cx).session().assists;
        assert!(assists.ideal_line);
        assert_eq!(assists.preset(), None);
    });
    assert!(cfg_file(&root, "assists.ini").contains("IDEAL_LINE=1"));

    act(cx, window, |window, cx| {
        window.click(("assist-preset", AssistPreset::Racer as usize), cx)
    });
    cx.update(|cx| {
        assert_eq!(
            game.read(cx).session().assists.preset(),
            Some(AssistPreset::Racer)
        )
    });

    // The next start picks up where this one left off.
    let (_, again, _) = open(cx, &root);
    cx.update(|cx| {
        assert_eq!(
            again.read(cx).session().assists.preset(),
            Some(AssistPreset::Racer)
        )
    });
}

#[gpui_kit::test]
fn controls_are_inverted_switched_and_saved_as_a_preset(cx: &mut TestAppContext) {
    let root = game_folder("controls-settings");
    std::fs::write(
        root.join("cfg/controls.ini"),
        "[HEADER]\nINPUT_METHOD=WHEEL\n[CONTROLLERS]\nCON0=Wheel Base\n\
         [STEER]\nJOY=0\nAXLE=0\nSCALE=1\n[THROTTLE]\nJOY=0\nAXLE=2\nMIN=-1\nMAX=1\n",
    )
    .unwrap();
    let (window, _, _) = open(cx, &root);
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click(CONTROLS_PAGE, cx));

    // Throttle is the second axis.
    act(cx, window, |window, cx| {
        window.click(("axis-invert", 1usize), cx)
    });
    let written = cfg_file(&root, "controls.ini");
    assert!(
        written.contains("[THROTTLE]\r\nJOY=0\r\nAXLE=2\r\nMIN=1\r\nMAX=-1"),
        "{written}"
    );

    act(cx, window, |window, cx| window.click("preset-save-as", cx));
    act(cx, window, |window, cx| window.input("Mine", cx));
    act(cx, window, |window, cx| window.click("preset-save", cx));
    let preset = Install::at(root.clone(), None)
        .cfg_dir()
        .join("controllers/savedsetups/Mine.ini");
    assert!(std::fs::read_to_string(preset).unwrap().contains("MIN=1"));

    act(cx, window, |window, cx| window.click("input-gamepad", cx));
    assert!(cfg_file(&root, "controls.ini").contains("INPUT_METHOD=X360"));
}

#[gpui_kit::test]
fn searching_settings_opens_the_page_that_has_it(cx: &mut TestAppContext) {
    let root = game_folder("settings-search");
    let (window, _, _) = open(cx, &root);
    act(cx, window, |window, cx| window.click(SETTINGS, cx));
    act(cx, window, |window, cx| window.click("settings-search", cx));
    act(cx, window, |window, cx| window.input("kerb", cx));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("page-controls").is_some());
        assert!(
            window.try_find(("Assetto Corsa", 0usize)).is_none(),
            "Video has no kerbs"
        );
    })
    .unwrap();
}
