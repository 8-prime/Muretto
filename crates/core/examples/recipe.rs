//! Installs a well-known mod from an archive you downloaded into a folder of
//! your choosing, uses it and removes it again, to check a recipe against the
//! real archive without touching the game:
//!
//! `cargo run -p muretto-core --example recipe -- pure ~/Downloads/Pure.zip /tmp/fake-game`

use std::{path::PathBuf, sync::Arc, time::Duration};

use muretto_core::{
    Install,
    packages::{Progress, Receipt, recipes},
};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(id), Some(archive), Some(root)) = (args.next(), args.next(), args.next()) else {
        panic!("usage: recipe <id> <archive> <folder>");
    };
    let recipe = recipes::ALL
        .iter()
        .find(|recipe| recipe.id == id)
        .expect("a known recipe id");
    let root = PathBuf::from(root);
    std::fs::create_dir_all(&root).unwrap();
    let install = Install::at(root.clone(), None);

    let progress = Progress::new();
    let watcher = {
        let progress = Arc::clone(&progress);
        std::thread::spawn(move || {
            while Arc::strong_count(&progress) > 1 {
                if let Some(fraction) = progress.fraction() {
                    eprint!("\r{:?} {:>3.0}%   ", progress.stage(), fraction * 100.);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
    };
    let installed = recipe.install_from_file(&install, archive.as_ref(), &progress);
    drop(progress);
    watcher.join().unwrap();
    eprintln!();
    println!("installed: {:?}", installed.unwrap());

    let receipt = Receipt::load(&root, recipe.id).expect("a receipt");
    println!("{} files, version {}", receipt.files.len(), receipt.version);
    let mut folders: Vec<_> = receipt
        .files
        .iter()
        .map(|file| file.splitn(4, '/').take(3).collect::<Vec<_>>().join("/"))
        .collect();
    folders.dedup();
    for folder in folders {
        println!("  {folder}");
    }

    recipe.use_now(&install).unwrap();
    println!("in use: {}", recipe.in_use(&install));
    recipe.remove(&install).unwrap();
    let left: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    println!("left after removal: {left:?}");
}
