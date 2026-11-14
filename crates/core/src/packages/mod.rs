//! Installing things into the game folder and remembering what was put
//! there, so that an update can clear out what the old version left behind
//! and a removal takes exactly what was added.
//!
//! A package is an archive laid out like the game folder. Installing one
//! unpacks it beside the game, checks it, moves the files into place and
//! writes a [`Receipt`] listing them.

pub mod csp;
pub mod recipes;
pub mod weather_fx;

use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed},
    },
    time::Duration,
};

use serde_json::{Value, json};

use crate::{Error, Result, io_err};

/// Muretto's own folder inside the game folder.
const STATE_DIR: &str = ".muretto";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Starting,
    Downloading,
    Unpacking,
    Installing,
}

/// A running installation, shared between the thread doing the work and the
/// interface showing it.
#[derive(Debug, Default)]
pub struct Progress {
    stage: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
    cancelled: AtomicBool,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn stage(&self) -> Stage {
        match self.stage.load(Relaxed) {
            1 => Stage::Downloading,
            2 => Stage::Unpacking,
            3 => Stage::Installing,
            _ => Stage::Starting,
        }
    }

    /// Bytes handled in the current stage.
    pub fn done(&self) -> u64 {
        self.done.load(Relaxed)
    }

    /// Bytes the current stage will handle, when known.
    pub fn total(&self) -> Option<u64> {
        Some(self.total.load(Relaxed)).filter(|&total| total > 0)
    }

    pub fn fraction(&self) -> Option<f32> {
        self.total()
            .map(|total| (self.done() as f32 / total as f32).min(1.))
    }

    /// Asks the work to stop. It does so at the next safe point; once files
    /// are being moved into the game folder it runs to completion.
    pub fn cancel(&self) {
        self.cancelled.store(true, Relaxed);
    }

    fn begin(&self, stage: Stage, total: Option<u64>) {
        self.done.store(0, Relaxed);
        self.total.store(total.unwrap_or(0), Relaxed);
        self.stage.store(stage as u8, Relaxed);
    }

    fn advance(&self, bytes: usize) -> Result<()> {
        self.done.fetch_add(bytes as u64, Relaxed);
        if self.cancelled.load(Relaxed) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

/// What a package put into the game folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    pub id: String,
    pub version: String,
    /// Paths relative to the game folder, with forward slashes.
    pub files: Vec<String>,
}

impl Receipt {
    fn path(root: &Path, id: &str) -> PathBuf {
        root.join(STATE_DIR)
            .join("packages")
            .join(format!("{id}.json"))
    }

    pub fn load(root: &Path, id: &str) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(root, id)).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Some(Receipt {
            id: id.to_owned(),
            version: value.get("version")?.as_str()?.to_owned(),
            files: value
                .get("files")?
                .as_array()?
                .iter()
                .filter_map(|file| file.as_str().map(str::to_owned))
                .collect(),
        })
    }

    fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path(root, &self.id);
        create_parent(&path)?;
        let value = json!({ "version": self.version, "files": self.files });
        std::fs::write(&path, format!("{value:#}\n")).map_err(io_err(&path))
    }
}

/// An archive unpacked beside the game, not yet part of it.
pub struct Staged {
    root: PathBuf,
    dir: PathBuf,
    /// Relative paths of everything unpacked, with forward slashes.
    pub files: Vec<String>,
}

impl Staged {
    /// Unpacks from `archive` every file for which `belongs` holds. Archives
    /// that wrap their contents in one top-level folder are unwrapped.
    pub fn unpack(
        root: &Path,
        id: &str,
        archive: &Path,
        belongs: impl Fn(&str) -> bool,
        progress: &Progress,
    ) -> Result<Self> {
        let unreadable = |e: zip::result::ZipError| {
            Error::Archive(format!(
                "{} is not a readable zip archive: {e}",
                archive.display()
            ))
        };
        let file = File::open(archive).map_err(io_err(archive))?;
        let mut zip = zip::ZipArchive::new(file).map_err(unreadable)?;

        // (index, path) of every file; a path that would leave the target
        // folder condemns the whole archive.
        let mut entries = Vec::new();
        let mut total = 0;
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(unreadable)?;
            if entry.is_dir() {
                continue;
            }
            let Some(path) = entry.enclosed_name() else {
                return Err(Error::Archive(format!(
                    "The archive holds a file that would be written outside the game folder: {}",
                    entry.name().unwrap_or_default()
                )));
            };
            let path = path
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            total += entry.size();
            entries.push((index, path));
        }

        let prefix =
            wrapper(entries.iter().map(|(_, path)| path.as_str()), &belongs).ok_or_else(|| {
                Error::Archive("Nothing in this archive belongs in the game folder.".to_owned())
            })?;

        let dir = root.join(STATE_DIR).join("staging").join(id);
        let _ = std::fs::remove_dir_all(&dir);
        let mut staged = Staged {
            root: root.to_path_buf(),
            dir,
            files: Vec::new(),
        };
        progress.begin(Stage::Unpacking, Some(total));
        let mut buffer = vec![0; 64 * 1024];
        for (index, path) in entries {
            let Some(relative) = path.strip_prefix(&prefix).filter(|p| belongs(p)) else {
                continue;
            };
            let mut entry = zip.by_index(index).map_err(unreadable)?;
            let target = staged.dir.join(relative);
            create_parent(&target)?;
            let mut out = File::create(&target).map_err(io_err(&target))?;
            loop {
                let read = entry
                    .read(&mut buffer)
                    .map_err(|e| Error::Archive(format!("{path} could not be unpacked: {e}")))?;
                if read == 0 {
                    break;
                }
                out.write_all(&buffer[..read]).map_err(io_err(&target))?;
                progress.advance(read)?;
            }
            staged.files.push(relative.to_owned());
        }
        Ok(staged)
    }

    pub fn contains(&self, file: &str) -> bool {
        self.files.iter().any(|f| f == file)
    }

    pub fn read(&self, file: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(file)).ok()
    }

    /// Moves the files into the game folder, replacing what is there, deletes
    /// whatever the previously installed version owned that this one does
    /// not, and writes the receipt.
    pub fn install(
        mut self,
        id: &str,
        version: &str,
        replaced: Replaced,
        progress: &Progress,
    ) -> Result<Receipt> {
        let previous = Receipt::load(&self.root, id);
        let owned: BTreeSet<&str> = previous
            .iter()
            .flat_map(|receipt| receipt.files.iter().map(String::as_str))
            .collect();
        let backups = backups_dir(&self.root, id);

        progress.begin(Stage::Installing, Some(self.files.len() as u64));
        for file in &self.files {
            let target = self.root.join(file);
            create_parent(&target)?;
            // A file that was here before this package ever was belongs to
            // the game or to another mod: set it aside, once, so removal can
            // put it back.
            let backup = backups.join(file);
            if replaced == Replaced::Kept
                && target.is_file()
                && !owned.contains(file.as_str())
                && !backup.exists()
            {
                create_parent(&backup)?;
                std::fs::rename(&target, &backup).map_err(io_err(&backup))?;
            }
            std::fs::rename(self.dir.join(file), &target).map_err(io_err(&target))?;
            // Past the point of no return: count, but ignore cancellation.
            let _ = progress.advance(1);
        }

        let files = std::mem::take(&mut self.files);
        if let Some(previous) = &previous {
            let kept: BTreeSet<&str> = files.iter().map(String::as_str).collect();
            let stale: Vec<&String> = previous
                .files
                .iter()
                .filter(|f| !kept.contains(f.as_str()))
                .collect();
            delete(&self.root, stale.iter().copied());
            // What this version no longer overwrites gets its original back.
            for file in stale {
                restore(&self.root, &backups, file);
            }
        }
        let receipt = Receipt {
            id: id.to_owned(),
            version: version.to_owned(),
            files,
        };
        receipt.save(&self.root)?;
        Ok(receipt)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        // Leave no empty staging folder behind in the game folder.
        if let Some(staging) = self.dir.parent() {
            let _ = std::fs::remove_dir(staging);
        }
    }
}

/// What becomes of files a package overwrites that it did not put there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced {
    /// Set aside and put back when the package is removed. For mods laid
    /// over the game's own folders.
    Kept,
    /// Overwritten for good. For a package that replaces an earlier copy of
    /// itself installed by other means.
    Discarded,
}

/// Deletes everything the package's receipt lists and the receipt, and puts
/// back the files it had overwritten. Returns whether there was a receipt.
pub fn uninstall(root: &Path, id: &str) -> Result<bool> {
    let Some(receipt) = Receipt::load(root, id) else {
        return Ok(false);
    };
    delete(root, receipt.files.iter());
    let backups = backups_dir(root, id);
    for file in &receipt.files {
        restore(root, &backups, file);
    }
    let _ = std::fs::remove_dir_all(&backups);
    if let Some(parent) = backups.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    let path = Receipt::path(root, id);
    std::fs::remove_file(&path).map_err(io_err(&path))?;
    // Leave no empty state folder behind once the last package is gone.
    let state = root.join(STATE_DIR);
    let _ = std::fs::remove_dir(state.join("packages"));
    let _ = std::fs::remove_dir(&state);
    Ok(true)
}

/// Downloads `url` to `to`. `size_hint` stands in for the total when the
/// server does not announce one.
pub fn download(url: &str, to: &Path, size_hint: Option<u64>, progress: &Progress) -> Result<()> {
    let failed = |e: &dyn std::fmt::Display| Error::Network(format!("Download failed: {e}"));
    progress.begin(Stage::Downloading, size_hint);
    let mut response = agent().get(url).call().map_err(|e| failed(&e))?;
    progress.begin(
        Stage::Downloading,
        response.body().content_length().or(size_hint),
    );

    create_parent(to)?;
    let partial = to.with_extension("part");
    let result = (|| {
        let mut out = File::create(&partial).map_err(io_err(&partial))?;
        let mut body = response.body_mut().as_reader();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let read = body.read(&mut buffer).map_err(|e| failed(&e))?;
            if read == 0 {
                break;
            }
            out.write_all(&buffer[..read]).map_err(io_err(&partial))?;
            progress.advance(read)?;
        }
        std::fs::rename(&partial, to).map_err(io_err(to))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// Fetches a page of text.
pub fn fetch_text(url: &str) -> Result<String> {
    let failed = |e: ureq::Error| Error::Network(format!("{url} could not be read: {e}"));
    agent()
        .get(url)
        .call()
        .map_err(failed)?
        .body_mut()
        .read_to_string()
        .map_err(failed)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(concat!("Muretto/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Where downloads wait before they are unpacked.
pub(crate) fn downloads_dir(root: &Path) -> PathBuf {
    root.join(STATE_DIR).join("downloads")
}

/// The folder prefix under which the archive keeps its game-folder layout:
/// empty for a plain archive, `"Name 1.2/"` for one wrapped in a folder.
fn wrapper<'a>(
    paths: impl Iterator<Item = &'a str> + Clone,
    belongs: impl Fn(&str) -> bool,
) -> Option<String> {
    if paths.clone().any(&belongs) {
        return Some(String::new());
    }
    paths.clone().find_map(|path| {
        let (folder, rest) = path.split_once('/')?;
        belongs(rest).then(|| format!("{folder}/"))
    })
}

fn backups_dir(root: &Path, id: &str) -> PathBuf {
    root.join(STATE_DIR).join("backups").join(id)
}

/// Moves the set-aside original of `file` back into the game folder, if
/// there is one.
fn restore(root: &Path, backups: &Path, file: &str) {
    let backup = backups.join(file);
    if backup.is_file() {
        let target = root.join(file);
        if create_parent(&target).is_ok() {
            let _ = std::fs::rename(&backup, &target);
        }
    }
}

/// Deletes files, then any folders that emptied.
fn delete<'a>(root: &Path, files: impl Iterator<Item = &'a String>) {
    for file in files {
        let path = root.join(file);
        let _ = std::fs::remove_file(&path);
        let mut folder = path.parent();
        while let Some(dir) = folder.filter(|dir| *dir != root && dir.starts_with(root)) {
            // Fails, and stops the climb, as soon as a folder has other contents.
            if std::fs::remove_dir(dir).is_err() {
                break;
            }
            folder = dir.parent();
        }
    }
}

fn create_parent(path: &Path) -> Result<()> {
    match path.parent() {
        Some(parent) => std::fs::create_dir_all(parent).map_err(io_err(parent)),
        None => Ok(()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("muretto-pkg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn zip_of(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, contents) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn in_game(path: &str) -> bool {
        path.starts_with("extension/") || path == "dwrite.dll"
    }

    #[test]
    fn an_update_replaces_files_and_clears_what_the_old_version_left() {
        let dir = temp_dir("update");
        let root = dir.join("game");
        std::fs::create_dir_all(root.join("extension/weather/pure")).unwrap();
        std::fs::write(root.join("extension/weather/pure/mine.lua"), "kept").unwrap();
        let progress = Progress::new();

        zip_of(
            &dir.join("one.zip"),
            &[
                ("dwrite.dll", "v1"),
                ("extension/lua/old/gone.lua", "v1"),
                ("extension/config/a.ini", "v1"),
                ("readme.txt", "not for the game folder"),
            ],
        );
        let staged =
            Staged::unpack(&root, "pkg", &dir.join("one.zip"), in_game, &progress).unwrap();
        assert!(staged.contains("dwrite.dll") && !staged.contains("readme.txt"));
        staged
            .install("pkg", "1", Replaced::Discarded, &progress)
            .unwrap();
        assert!(!root.join("readme.txt").exists());

        zip_of(
            &dir.join("two.zip"),
            &[("dwrite.dll", "v2"), ("extension/config/a.ini", "v2")],
        );
        let receipt = Staged::unpack(&root, "pkg", &dir.join("two.zip"), in_game, &progress)
            .unwrap()
            .install("pkg", "2", Replaced::Discarded, &progress)
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("dwrite.dll")).unwrap(),
            "v2"
        );
        assert!(
            !root.join("extension/lua").exists(),
            "the old version's folder is gone"
        );
        assert!(
            root.join("extension/weather/pure/mine.lua").exists(),
            "other files are untouched"
        );
        assert_eq!(Receipt::load(&root, "pkg"), Some(receipt));
        assert!(!root.join(".muretto/staging").exists());

        assert!(uninstall(&root, "pkg").unwrap());
        assert!(!root.join("dwrite.dll").exists() && !root.join("extension/config").exists());
        assert!(root.join("extension/weather/pure/mine.lua").exists());
        assert!(!uninstall(&root, "pkg").unwrap());
        assert!(!root.join(".muretto").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_a_mod_overwrites_come_back_when_it_goes() {
        let dir = temp_dir("backups");
        let root = dir.join("game");
        std::fs::create_dir_all(root.join("system/cfg/ppfilters")).unwrap();
        std::fs::write(root.join("system/cfg/ppfilters/default.ini"), "stock").unwrap();
        let progress = Progress::new();
        let overlay = |path: &str| path.starts_with("system/");
        let install = |archive: &str, version: &str| {
            Staged::unpack(&root, "mod", &dir.join(archive), overlay, &progress)
                .unwrap()
                .install("mod", version, Replaced::Kept, &progress)
                .unwrap()
        };

        zip_of(
            &dir.join("one.zip"),
            &[
                ("system/cfg/ppfilters/default.ini", "modded 1"),
                ("system/cfg/ppfilters/mod.ini", "new"),
            ],
        );
        install("one.zip", "1");
        // Updating must not mistake the mod's own file for the original.
        zip_of(
            &dir.join("two.zip"),
            &[("system/cfg/ppfilters/default.ini", "modded 2")],
        );
        install("two.zip", "2");
        let read =
            |file: &str| std::fs::read_to_string(root.join("system/cfg/ppfilters").join(file));
        assert_eq!(read("default.ini").unwrap(), "modded 2");
        assert!(read("mod.ini").is_err());

        assert!(uninstall(&root, "mod").unwrap());
        assert_eq!(read("default.ini").unwrap(), "stock");
        assert!(!root.join(".muretto/backups").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archives_packed_by_7zip_are_read() {
        // Pure 3.21 is packed with LZMA; 7-Zip also offers bzip2. Written by
        // Python's zipfile, as the zip crate cannot write LZMA.
        let dir = temp_dir("methods");
        for (name, bytes) in [
            ("lzma", &include_bytes!("testdata/lzma.zip")[..]),
            ("bzip2", &include_bytes!("testdata/bzip2.zip")[..]),
        ] {
            let archive = dir.join(format!("{name}.zip"));
            std::fs::write(&archive, bytes).unwrap();
            let staged = Staged::unpack(
                &dir.join("game"),
                "pkg",
                &archive,
                in_game,
                &Progress::new(),
            )
            .unwrap();
            assert_eq!(staged.read("extension/a.lua").unwrap(), b"packed", "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_wrapping_folder_is_unwrapped() {
        let dir = temp_dir("wrapped");
        zip_of(
            &dir.join("w.zip"),
            &[
                ("Patch 1.0/dwrite.dll", "x"),
                ("Patch 1.0/extension/a", "y"),
            ],
        );
        let staged = Staged::unpack(
            &dir.join("game"),
            "pkg",
            &dir.join("w.zip"),
            in_game,
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(staged.files, ["dwrite.dll", "extension/a"]);
        drop(staged);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unrelated_and_escaping_archives_are_refused_before_anything_is_written() {
        let dir = temp_dir("refused");
        let root = dir.join("game");
        zip_of(&dir.join("other.zip"), &[("photos/cat.jpg", "meow")]);
        let other = Staged::unpack(
            &root,
            "pkg",
            &dir.join("other.zip"),
            in_game,
            &Progress::new(),
        );
        assert!(matches!(other, Err(Error::Archive(m)) if m.contains("Nothing in this archive")));

        zip_of(
            &dir.join("escape.zip"),
            &[("extension/a", "fine"), ("../outside.txt", "not fine")],
        );
        let escape = Staged::unpack(
            &root,
            "pkg",
            &dir.join("escape.zip"),
            |_| true,
            &Progress::new(),
        );
        assert!(matches!(escape, Err(Error::Archive(m)) if m.contains("outside the game folder")));
        assert!(!dir.join("outside.txt").exists() && !root.join("extension").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelling_stops_unpacking_and_leaves_the_game_folder_alone() {
        let dir = temp_dir("cancel");
        let root = dir.join("game");
        zip_of(&dir.join("a.zip"), &[("dwrite.dll", "x")]);
        let progress = Progress::new();
        progress.cancel();
        let result = Staged::unpack(&root, "pkg", &dir.join("a.zip"), in_game, &progress);
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!root.join("dwrite.dll").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
