//! Drives the real window with pointer and keyboard events against a small
//! game folder built on disk, and checks what the session becomes.

use std::path::{Path, PathBuf};

use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    prelude::*,
    px, size,
    test::{TestQueryExt as _, TestWindowExt as _},
};
use muretto::{
    game::{Game, Launch},
    shell::Shell,
};
use muretto_core::{Install, Library, session::SessionMode};

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

fn open(cx: &mut TestAppContext, root: &Path) -> (AnyWindowHandle, Entity<Game>, Entity<Shell>) {
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
            |window, cx| cx.new(|cx| Shell::with_content(install, library, window, cx)),
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
