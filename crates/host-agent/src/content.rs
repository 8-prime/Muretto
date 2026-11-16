//! The car and track files the server reads, as uploaded by Muretto.
//!
//! They live where AssettoServer looks for them, under its working
//! directory. `muretto_host_protocol::content::classify` decides what may be
//! stored and where; nothing else is written.

use std::collections::HashMap;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use axum::body::Bytes;
use futures_util::{Stream, StreamExt};
use muretto_host_protocol::content::{
    ContentPath, FileKind, Installed, MAX_FILE, Manifest, UploadPlan, classify,
};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::store::temp_path;

/// Paths below the server's directory never go deeper than
/// `content/tracks/{track}/{layout}/ai/fast_lane.ai`.
const MAX_DEPTH: usize = 6;

pub struct Content {
    root: PathBuf,
    /// SHA-256 of files already hashed, while their size and time stay put.
    hashes: Mutex<HashMap<PathBuf, (u64, SystemTime, String)>>,
}

#[derive(Debug)]
pub enum UploadError {
    TooLarge,
    /// The body's hash is not the one announced.
    Mismatch,
    Body(String),
    Io(io::Error),
}

impl Content {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            hashes: Mutex::new(HashMap::new()),
        }
    }

    /// Every server file present.
    pub fn files(&self) -> Vec<FileKind> {
        let mut found = Vec::new();
        for top in ["content", "system"] {
            walk(&self.root.join(top), top, 1, &mut found);
        }
        found
    }

    pub fn installed(&self) -> Installed {
        Installed::from_files(&self.files())
    }

    /// The files in `manifest` the host lacks or holds another copy of.
    pub fn plan(&self, manifest: &Manifest) -> io::Result<UploadPlan> {
        let mut upload = Vec::new();
        for file in &manifest.files {
            let path = self.path(&file.path);
            let same = match std::fs::metadata(&path) {
                Ok(meta) if meta.len() == file.size => self.hash(&path, &meta)? == file.sha256,
                Ok(_) => false,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };
            if !same {
                upload.push(file.path.clone());
            }
        }
        Ok(UploadPlan { upload })
    }

    /// Stores `body` at `path` if it hashes to `sha256`, replacing what was
    /// there in one step. Nothing changes otherwise.
    pub async fn upload<S, E>(
        &self,
        path: &ContentPath,
        sha256: &str,
        mut body: S,
    ) -> Result<(), UploadError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        let target = self.path(path);
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(UploadError::Io)?;
        }
        let temp = temp_path(&target);
        let result = async {
            let mut file = tokio::fs::File::create(&temp)
                .await
                .map_err(UploadError::Io)?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(|error| UploadError::Body(error.to_string()))?;
                size += chunk.len() as u64;
                if size > MAX_FILE {
                    return Err(UploadError::TooLarge);
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await.map_err(UploadError::Io)?;
            }
            if hex(&hasher.finalize()) != sha256 {
                return Err(UploadError::Mismatch);
            }
            file.sync_all().await.map_err(UploadError::Io)?;
            drop(file);
            tokio::fs::rename(&temp, &target)
                .await
                .map_err(UploadError::Io)
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temp).await;
        }
        result
    }

    fn path(&self, path: &ContentPath) -> PathBuf {
        // `classify` only lets through relative paths without `..`.
        self.root.join(path.as_str())
    }

    fn hash(&self, path: &Path, meta: &std::fs::Metadata) -> io::Result<String> {
        let modified = meta.modified()?;
        let mut hashes = self.hashes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((size, time, hash)) = hashes.get(path)
            && *size == meta.len()
            && *time == modified
        {
            return Ok(hash.clone());
        }
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 16];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let hash = hex(&hasher.finalize());
        hashes.insert(path.to_owned(), (meta.len(), modified, hash.clone()));
        Ok(hash)
    }
}

fn walk(dir: &Path, relative: &str, depth: usize, found: &mut Vec<FileKind>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = format!("{relative}/{name}");
        if kind.is_dir() && depth < MAX_DEPTH {
            walk(&entry.path(), &path, depth + 1, found);
        } else if kind.is_file()
            && let Some(file) = classify(&path)
            // Only files at the very path the server reads count: a
            // `Models.ini` is not the `models.ini` it looks for.
            && file.path.as_str() == path
        {
            found.push(file.kind);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use muretto_host_protocol::content::ManifestFile;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("muretto-agent-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn body(bytes: &'static [u8]) -> impl Stream<Item = Result<Bytes, String>> + Unpin {
        futures_util::stream::iter(
            bytes
                .chunks(3)
                .map(|chunk| Ok(Bytes::from_static(chunk)))
                .collect::<Vec<_>>(),
        )
    }

    fn path(path: &str) -> ContentPath {
        classify(path).unwrap().path
    }

    #[tokio::test]
    async fn uploads_land_where_the_server_reads_them() {
        let dir = Dir::new("upload");
        let content = Content::new(dir.0.clone());
        let data = path("content/cars/ks_audi_r8_lms/data.acd");
        content
            .upload(&data, &sha(b"car data"), body(b"car data"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(dir.0.join("content/cars/ks_audi_r8_lms/data.acd")).unwrap(),
            b"car data"
        );
        assert_eq!(content.installed().cars[0].id, "ks_audi_r8_lms");
    }

    #[tokio::test]
    async fn a_body_that_does_not_match_its_hash_changes_nothing() {
        let dir = Dir::new("mismatch");
        let content = Content::new(dir.0.clone());
        let data = path("content/cars/x/data.acd");
        content
            .upload(&data, &sha(b"old"), body(b"old"))
            .await
            .unwrap();
        let result = content.upload(&data, &sha(b"new"), body(b"tampered")).await;
        assert!(matches!(result, Err(UploadError::Mismatch)));
        assert_eq!(
            std::fs::read(dir.0.join("content/cars/x/data.acd")).unwrap(),
            b"old"
        );
        let leftovers = std::fs::read_dir(dir.0.join("content/cars/x"))
            .unwrap()
            .count();
        assert_eq!(leftovers, 1);
    }

    #[tokio::test]
    async fn the_plan_asks_for_missing_and_changed_files() {
        let dir = Dir::new("plan");
        let content = Content::new(dir.0.clone());
        let same = path("content/cars/a/data.acd");
        let changed = path("content/cars/b/data.acd");
        let missing = path("content/tracks/t/data/surfaces.ini");
        content
            .upload(&same, &sha(b"same"), body(b"same"))
            .await
            .unwrap();
        content
            .upload(&changed, &sha(b"old!"), body(b"old!"))
            .await
            .unwrap();
        let manifest = Manifest {
            files: [(&same, b"same"), (&changed, b"new!"), (&missing, b"surf")]
                .into_iter()
                .map(|(path, bytes)| ManifestFile {
                    path: path.clone(),
                    size: bytes.len() as u64,
                    sha256: sha(bytes),
                })
                .collect(),
        };
        assert_eq!(content.plan(&manifest).unwrap().upload, [changed, missing]);
    }

    #[test]
    fn files_in_the_wrong_case_do_not_count() {
        let dir = Dir::new("case");
        let track = dir.0.join("content/tracks/t");
        std::fs::create_dir_all(track.join("data")).unwrap();
        std::fs::write(track.join("data/Surfaces.ini"), "").unwrap();
        assert_eq!(Content::new(dir.0.clone()).files(), []);
        std::fs::write(track.join("data/surfaces.ini"), "").unwrap();
        assert_eq!(Content::new(dir.0.clone()).installed().tracks[0].id, "t");
    }
}
