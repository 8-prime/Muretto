//! Downloads a Custom Shaders Patch build from the author's site and installs
//! it into a folder of your choosing, to check the whole path against the
//! real download without touching the game:
//!
//! `cargo run -p muretto-core --example csp -- /tmp/fake-game`

use std::{sync::Arc, time::Duration};

use muretto_core::{
    Install,
    packages::{
        Progress,
        csp::{self, Source as _},
    },
};

fn main() {
    let root = std::env::args().nth(1).expect("a folder to install into");
    std::fs::create_dir_all(&root).unwrap();
    let install = Install::at(root.into(), None);

    let catalogue = csp::Official
        .catalogue()
        .expect("the download page is readable");
    println!("{} builds listed", catalogue.releases.len());
    for release in catalogue.releases.iter().take(4) {
        println!(
            "  {:<8} {:?} {:?}",
            release.version, release.standing, release.size_bytes
        );
    }
    let release = catalogue
        .recommended()
        .expect("a recommended build")
        .clone();

    let progress = Progress::new();
    let watcher = {
        let progress = Arc::clone(&progress);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                eprintln!(
                    "  {:?} {:?} of {:?}",
                    progress.stage(),
                    progress.done(),
                    progress.total()
                );
                if Arc::strong_count(&progress) == 1 {
                    break;
                }
            }
        })
    };
    let result = csp::install(&install, &csp::Official, &release, &progress);
    drop(progress);
    watcher.join().unwrap();
    println!("{result:#?}");
}
