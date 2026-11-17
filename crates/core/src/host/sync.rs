//! Putting a preset's content on a host: the few files the server reads
//! from each of its cars and its track, taken from the player's own install.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use muretto_host_protocol::{
    content::{ContentPath, FileKind, Manifest, ManifestFile, classify},
    presets::Preset,
};
use sha2::{Digest, Sha256};

use super::{Client, HostError, HostResult};
use crate::{Install, packages::Progress, packages::Stage};

/// A file in the player's install that the server reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub path: ContentPath,
    pub file: PathBuf,
    pub size: u64,
    pub sha256: String,
}

/// What [`sync`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Synced {
    /// Files sent to the host.
    pub uploaded: usize,
    /// Cars and the track that this install cannot give the host the
    /// server's files for, by folder name.
    pub missing: Vec<String>,
}

/// The files of `preset`'s cars and track, and the game's own surfaces,
/// found in `install`, with what the install lacks of them by folder name.
/// Track files of other layouts are left out, and so are the track's models
/// unless `track_models`: the server only checks drivers' copies against
/// them, and they run to hundreds of megabytes.
pub fn local_files(
    install: &Install,
    preset: &Preset,
    track_models: bool,
) -> (Vec<String>, Vec<(ContentPath, PathBuf)>) {
    type Found = Vec<(FileKind, ContentPath, PathBuf)>;
    fn add(found: &mut Found, relative: String, file: PathBuf) {
        if let Some(server) = classify(&relative) {
            found.push((server.kind, server.path, file));
        }
    }
    let mut missing = Vec::new();
    let mut found = Found::new();

    // Optional: without it the server skips that check.
    let surfaces = install.root.join("system/data/surfaces.ini");
    if surfaces.is_file() {
        add(&mut found, "system/data/surfaces.ini".to_owned(), surfaces);
    }
    for car in preset.cars() {
        let mut files = Found::new();
        for (name, file) in files_in(&install.cars_dir().join(car)) {
            add(&mut files, format!("content/cars/{car}/{name}"), file);
        }
        if !files
            .iter()
            .any(|(kind, ..)| matches!(kind, FileKind::CarData { .. }))
        {
            missing.push(car.to_owned());
        }
        found.extend(files);
    }

    let track = &preset.track;
    let mut files = Found::new();
    walk(
        &install.tracks_dir().join(&track.id),
        "",
        3,
        &mut |relative, file| {
            add(
                &mut files,
                format!("content/tracks/{}/{relative}", track.id),
                file,
            );
        },
    );
    // Only the preset's layout: the server reads no other.
    files.retain(|(kind, path, _)| match kind {
        FileKind::TrackSurfaces { layout, .. }
        | FileKind::TrackModels { layout, .. }
        | FileKind::AiSpline { layout, .. } => layout == &track.layout,
        FileKind::TrackRoot { .. } => track_models || !is_model(path),
        _ => true,
    });
    let layout_found = files.iter().any(|(kind, ..)| {
        matches!(
            kind,
            FileKind::TrackSurfaces { .. } | FileKind::TrackModels { .. }
        )
    });
    if !layout_found {
        missing.push(match &track.layout {
            Some(layout) => format!("{} ({layout})", track.id),
            None => track.id.clone(),
        });
    }
    found.extend(files);

    let files = found
        .into_iter()
        .map(|(_, path, file)| (path, file))
        .collect();
    (missing, files)
}

/// Whether `path` is a `.kn5` model.
fn is_model(path: &ContentPath) -> bool {
    path.as_str().to_ascii_lowercase().ends_with(".kn5")
}

/// Files directly in `dir`, by name.
fn files_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect()
}

/// Every file under `dir` down to `depth` folders, as a `/` path from it.
fn walk(dir: &Path, prefix: &str, depth: usize, found: &mut impl FnMut(String, PathBuf)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let relative = format!("{prefix}{name}");
        if kind.is_file() {
            found(relative, entry.path());
        } else if kind.is_dir() && depth > 0 {
            walk(&entry.path(), &format!("{relative}/"), depth - 1, found);
        }
    }
}

fn hash(file: &Path, progress: &Progress) -> HostResult<(u64, String)> {
    let failed = |e: std::io::Error| HostError::Unreachable(format!("{}: {e}", file.display()));
    let mut reader = std::fs::File::open(file).map_err(failed)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut size = 0;
    loop {
        let read = reader.read(&mut buffer).map_err(failed)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
        progress.advance(read).map_err(|_| HostError::Cancelled)?;
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((size, hex))
}

/// Counts what is read towards the progress, and stops when it is
/// cancelled.
struct Counted<'a, R> {
    inner: R,
    progress: &'a Progress,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.progress
            .advance(read)
            .map_err(|_| std::io::Error::other("cancelled"))?;
        Ok(read)
    }
}

/// Uploads whatever of `preset`'s content the host lacks or holds another
/// copy of; the track's models only with `track_models`.
pub fn sync(
    client: &Client,
    install: &Install,
    preset: &Preset,
    track_models: bool,
    progress: &Progress,
) -> HostResult<Synced> {
    let (missing, paths) = local_files(install, preset, track_models);
    let mut files = Vec::with_capacity(paths.len());
    progress.begin(Stage::Starting, None);
    for (path, file) in paths {
        let (size, sha256) = hash(&file, progress)?;
        files.push(LocalFile {
            path,
            file,
            size,
            sha256,
        });
    }
    let manifest = Manifest {
        files: files
            .iter()
            .map(|file| ManifestFile {
                path: file.path.clone(),
                size: file.size,
                sha256: file.sha256.clone(),
            })
            .collect(),
    };
    let plan = client.plan(&manifest)?;
    let wanted: Vec<&LocalFile> = files
        .iter()
        .filter(|file| plan.upload.contains(&file.path))
        .collect();
    progress.begin(
        Stage::Uploading,
        Some(wanted.iter().map(|file| file.size).sum()),
    );
    for file in &wanted {
        let reader = std::fs::File::open(&file.file)
            .map_err(|e| HostError::Unreachable(format!("{}: {e}", file.file.display())))?;
        let body = Counted {
            inner: reader,
            progress,
        };
        if let Err(error) = client.upload(&file.path, &file.sha256, body) {
            if progress.advance(0).is_err() {
                return Err(HostError::Cancelled);
            }
            return Err(error);
        }
    }
    Ok(Synced {
        uploaded: wanted.len(),
        missing,
    })
}
