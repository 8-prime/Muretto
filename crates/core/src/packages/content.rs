//! Cars and tracks, from however their authors packed them: a zip or 7z
//! archive, or a folder the player unpacked themselves. Archives hold one
//! car or a pack of dozens, laid out as the game folder or not, so the
//! contents are searched for anything that looks like a car or a track.
//!
//! Content already in the game folder is never touched: the player's copy
//! may be newer or modified.

use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use super::{Progress, Receipt, STATE_DIR, Stage, create_parent};
use crate::{Error, Install, Result, io_err, online::offers::Kind};

/// What an installation added and what it left alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Added {
    pub cars: Vec<String>,
    pub tracks: Vec<String>,
    /// Folder names of content the game folder already had.
    pub kept: Vec<String>,
}

impl Added {
    pub fn is_empty(&self) -> bool {
        self.cars.is_empty() && self.tracks.is_empty()
    }
}

/// The receipt id for a car or track Muretto installed.
pub fn receipt_id(kind: Kind, id: &str) -> String {
    match kind {
        Kind::Car => format!("car-{id}"),
        Kind::Track => format!("track-{id}"),
    }
}

/// Installs every car and track found in `source`, an archive or a folder.
pub fn install_content(install: &Install, source: &Path, progress: &Progress) -> Result<Added> {
    let root = &install.root;
    let staging = root.join(STATE_DIR).join("staging").join("content");
    let _ = std::fs::remove_dir_all(&staging);

    let from_folder = source.is_dir();
    let result = (|| {
        let searched = if from_folder {
            source.to_path_buf()
        } else {
            unpack(source, &staging, progress)?;
            staging.clone()
        };
        let fallback = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let items = find(&searched, &fallback);
        if items.is_empty() {
            return Err(Error::Archive(
                "No car or track was found in it. A car's folder has a data.acd or ui/ui_car.json; a track's has ui_track.json under ui."
                    .to_owned(),
            ));
        }

        let mut added = Added::default();
        progress.begin(Stage::Installing, Some(items.len() as u64));
        for (kind, name, dir) in items {
            let folder = match kind {
                Kind::Car => "cars",
                Kind::Track => "tracks",
            };
            let relative = format!("content/{folder}/{name}");
            let target = root.join(&relative);
            if target.exists() {
                added.kept.push(name);
                continue;
            }
            create_parent(&target)?;
            let mut files = Vec::new();
            list_files(&dir, &relative, &mut files);
            if from_folder {
                copy_dir(&dir, &target)?;
            } else {
                std::fs::rename(&dir, &target).map_err(io_err(&target))?;
            }
            Receipt {
                id: receipt_id(kind, &name),
                version: String::new(),
                files,
            }
            .save(root)?;
            match kind {
                Kind::Car => added.cars.push(name),
                Kind::Track => added.tracks.push(name),
            }
            let _ = progress.advance(1);
        }
        Ok(added)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    if let Some(parent) = staging.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    result
}

/// Downloads what a link points straight at and installs it.
pub fn install_offer(install: &Install, url: &str, progress: &Progress) -> Result<Added> {
    let name = crate::online::offers::archive_name(url)
        .filter(|name| enclosed(name).is_some_and(|p| p.components().count() == 1))
        .unwrap_or_else(|| "content.zip".to_owned());
    let archive = super::downloads_dir(&install.root).join(name);
    super::download(url, &archive, None, progress)?;
    let result = install_content(install, &archive, progress);
    let _ = std::fs::remove_file(&archive);
    if let Some(dir) = archive.parent() {
        let _ = std::fs::remove_dir(dir);
    }
    result
}

/// Cars and tracks under `dir`: (kind, folder name, where). A car or track
/// at the very top takes `fallback` as its name.
fn find(dir: &Path, fallback: &str) -> Vec<(Kind, String, PathBuf)> {
    let mut found = Vec::new();
    let mut queue = vec![(dir.to_path_buf(), 0)];
    while let Some((current, depth)) = queue.pop() {
        if let Some(kind) = kind_of(&current) {
            let name = if current == dir {
                fallback.to_owned()
            } else {
                current
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            if !name.is_empty() {
                found.push((kind, name, current));
            }
            continue;
        }
        if depth >= 8 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let skipped = entry.file_name() == "__MACOSX";
            if path.is_dir() && !skipped {
                queue.push((path, depth + 1));
            }
        }
    }
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}

fn kind_of(dir: &Path) -> Option<Kind> {
    if dir.join("data.acd").is_file() || dir.join("ui/ui_car.json").is_file() {
        return Some(Kind::Car);
    }
    let ui = dir.join("ui");
    if ui.join("ui_track.json").is_file() {
        return Some(Kind::Track);
    }
    // A track with layouts keeps one ui_track.json per layout.
    let has_layouts = std::fs::read_dir(&ui)
        .ok()?
        .flatten()
        .any(|layout| layout.path().join("ui_track.json").is_file());
    has_layouts.then_some(Kind::Track)
}

/// Every file under `dir`, as paths under `prefix`.
fn list_files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let relative = format!("{prefix}/{name}");
        if path.is_dir() {
            list_files(&path, &relative, out);
        } else {
            out.push(relative);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(io_err(to))?;
    for entry in std::fs::read_dir(from).map_err(io_err(from))?.flatten() {
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if source.is_dir() {
            copy_dir(&source, &target)?;
        } else {
            std::fs::copy(&source, &target).map_err(io_err(&target))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    SevenZip,
    Rar,
}

fn format_of(archive: &Path) -> Result<Format> {
    let mut magic = [0; 6];
    let read = File::open(archive)
        .and_then(|mut file| file.read(&mut magic))
        .map_err(io_err(archive))?;
    match &magic[..read] {
        [b'P', b'K', 3, 4, ..] | [b'P', b'K', 5, 6, ..] => Ok(Format::Zip),
        [b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c] => Ok(Format::SevenZip),
        [b'R', b'a', b'r', b'!', ..] => Ok(Format::Rar),
        _ => Err(Error::Archive(format!(
            "{} is not a zip or 7z archive.",
            archive.display()
        ))),
    }
}

/// A path inside an archive, if it stays inside the folder it is unpacked to.
fn enclosed(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    path.components()
        .all(|part| matches!(part, Component::Normal(_)))
        .then(|| path.to_path_buf())
        .filter(|path| !path.as_os_str().is_empty())
}

fn unpack(archive: &Path, to: &Path, progress: &Progress) -> Result<()> {
    std::fs::create_dir_all(to).map_err(io_err(to))?;
    match format_of(archive)? {
        Format::Zip => unzip(archive, to, progress),
        Format::SevenZip => un7z(archive, to, progress),
        Format::Rar => Err(Error::Archive(
            "Muretto cannot open RAR archives. Unpack it with another program and give Muretto the folder instead."
                .to_owned(),
        )),
    }
}

fn unzip(archive: &Path, to: &Path, progress: &Progress) -> Result<()> {
    let unreadable = |e: zip::result::ZipError| {
        Error::Archive(format!("{} could not be read: {e}", archive.display()))
    };
    let file = File::open(archive).map_err(io_err(archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(unreadable)?;
    let total = (0..zip.len())
        .filter_map(|index| zip.by_index_raw(index).ok().map(|entry| entry.size()))
        .sum();
    progress.begin(Stage::Unpacking, Some(total));
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(unreadable)?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            return Err(outside(&entry.name().unwrap_or_default()));
        };
        let target = to.join(path);
        write_entry(&mut entry, &target, progress)?;
    }
    Ok(())
}

fn un7z(archive: &Path, to: &Path, progress: &Progress) -> Result<()> {
    let unreadable = |e: sevenz_rust2::Error| {
        Error::Archive(format!("{} could not be read: {e}", archive.display()))
    };
    let file = File::open(archive).map_err(io_err(archive))?;
    let mut reader = sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty())
        .map_err(unreadable)?;
    let total = reader
        .archive()
        .files
        .iter()
        .map(|entry| entry.size())
        .sum();
    progress.begin(Stage::Unpacking, Some(total));
    // The reader's callback can only fail with the reader's own error;
    // ours waits here.
    let mut failed = None;
    reader
        .for_each_entries_with_decision(|entry, data| {
            use sevenz_rust2::EntryDecision;
            if entry.is_directory() || !entry.has_stream {
                return Ok(EntryDecision::Continue);
            }
            let result = match enclosed(entry.name()) {
                Some(path) => write_entry(data, &to.join(path), progress),
                None => Err(outside(entry.name())),
            };
            match result {
                Ok(()) => Ok(EntryDecision::Continue),
                Err(error) => {
                    failed = Some(error);
                    Ok(EntryDecision::Stop)
                }
            }
        })
        .map_err(unreadable)?;
    failed.map_or(Ok(()), Err)
}

fn outside(name: &str) -> Error {
    Error::Archive(format!(
        "The archive holds a file that would be written outside the game folder: {name}"
    ))
}

fn write_entry(data: &mut dyn Read, target: &Path, progress: &Progress) -> Result<()> {
    create_parent(target)?;
    let mut out = File::create(target).map_err(io_err(target))?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = data.read(&mut buffer).map_err(|e| {
            Error::Archive(format!("{} could not be unpacked: {e}", target.display()))
        })?;
        if read == 0 {
            return Ok(());
        }
        out.write_all(&buffer[..read]).map_err(io_err(target))?;
        progress.advance(read)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packages::{
        tests::{temp_dir, zip_of},
        uninstall,
    };

    fn game(dir: &Path) -> Install {
        let root = dir.join("game");
        std::fs::create_dir_all(root.join("content/cars/already_here")).unwrap();
        std::fs::write(root.join("content/cars/already_here/data.acd"), "mine").unwrap();
        Install::at(root, None)
    }

    #[test]
    fn a_pack_in_any_layout_adds_its_cars_and_tracks_and_leaves_existing_ones() {
        let dir = temp_dir("content-pack");
        let install = game(&dir);
        zip_of(
            &dir.join("pack.zip"),
            &[
                // Wrapped in the game folder's layout…
                ("Pack v2/content/cars/new_car/data.acd", "acd"),
                ("Pack v2/content/cars/new_car/skins/red/livery.png", "png"),
                ("Pack v2/content/cars/already_here/data.acd", "theirs"),
                // …or loose, with layouts.
                ("Track/new_track/ui/gp/ui_track.json", "{}"),
                ("Track/new_track/new_track.kn5", "kn5"),
                ("readme.txt", "hi"),
                ("__MACOSX/content/cars/._junk/data.acd", ""),
            ],
        );
        let added = install_content(&install, &dir.join("pack.zip"), &Progress::new()).unwrap();
        assert_eq!(added.cars, ["new_car"]);
        assert_eq!(added.tracks, ["new_track"]);
        assert_eq!(added.kept, ["already_here"]);
        let root = &install.root;
        assert!(
            root.join("content/cars/new_car/skins/red/livery.png")
                .is_file()
        );
        assert!(
            root.join("content/tracks/new_track/new_track.kn5")
                .is_file()
        );
        assert_eq!(
            std::fs::read_to_string(root.join("content/cars/already_here/data.acd")).unwrap(),
            "mine"
        );
        assert!(!root.join(".muretto/staging").exists());

        // Muretto knows what it added, so it can take it away again.
        assert!(uninstall(root, &receipt_id(Kind::Car, "new_car")).unwrap());
        assert!(!root.join("content/cars/new_car").exists());
        assert!(root.join("content/cars/already_here").exists());
    }

    #[test]
    fn a_folder_is_copied_and_a_lone_car_takes_the_archive_name() {
        let dir = temp_dir("content-folder");
        let install = game(&dir);
        let folder = dir.join("unpacked/some_car");
        std::fs::create_dir_all(folder.join("ui")).unwrap();
        std::fs::write(folder.join("ui/ui_car.json"), "{}").unwrap();
        let added = install_content(&install, &dir.join("unpacked"), &Progress::new()).unwrap();
        assert_eq!(added.cars, ["some_car"]);
        assert!(folder.exists(), "the player's folder is left as it was");

        zip_of(&dir.join("lone_car.zip"), &[("data.acd", "acd")]);
        let added = install_content(&install, &dir.join("lone_car.zip"), &Progress::new()).unwrap();
        assert_eq!(added.cars, ["lone_car"]);
    }

    #[test]
    fn archives_without_content_and_rar_archives_are_refused() {
        let dir = temp_dir("content-refused");
        let install = game(&dir);
        zip_of(&dir.join("notes.zip"), &[("notes/readme.txt", "hi")]);
        let error = install_content(&install, &dir.join("notes.zip"), &Progress::new());
        assert!(matches!(error, Err(Error::Archive(m)) if m.contains("No car or track")));
        std::fs::write(dir.join("car.rar"), b"Rar!\x1a\x07\x01\x00").unwrap();
        let error = install_content(&install, &dir.join("car.rar"), &Progress::new());
        assert!(matches!(error, Err(Error::Archive(m)) if m.contains("RAR")));
    }

    #[test]
    fn seven_zip_archives_are_unpacked() {
        let dir = temp_dir("content-7z");
        let install = game(&dir);
        let source = dir.join("src/cars/seven_car");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("data.acd"), "acd").unwrap();
        sevenz_rust2::compress_to_path(dir.join("src"), dir.join("seven.7z")).unwrap();
        let added = install_content(&install, &dir.join("seven.7z"), &Progress::new()).unwrap();
        assert_eq!(added.cars, ["seven_car"]);
        assert_eq!(
            std::fs::read_to_string(install.root.join("content/cars/seven_car/data.acd")).unwrap(),
            "acd"
        );
    }
}
