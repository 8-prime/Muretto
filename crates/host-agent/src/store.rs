//! Presets on disk: one JSON file each under `presets/`, and the one the
//! server runs as `active.json`.
//!
//! The active preset is a copy of what was activated, not a pointer: saving
//! the preset afterwards does not change what a restart of the agent brings
//! back up, and deleting it does not take the server down.

use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use muretto_host_protocol::presets::{Preset, PresetId, StoredPreset};

pub struct Store {
    dir: PathBuf,
    presets: BTreeMap<PresetId, StoredPreset>,
    active: Option<StoredPreset>,
}

/// What a save expects to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// `If-None-Match: *`: no preset with this id.
    Nothing,
    /// `If-Match: <revision>`.
    Revision(u64),
}

#[derive(Debug)]
pub enum SaveError {
    /// Neither header was sent.
    Unconditional,
    /// The preset is not as expected; carries its revision, if it exists.
    Conflict(Option<u64>),
    Io(io::Error),
}

impl Store {
    /// Reads what an earlier run left. A preset file that cannot be read is
    /// reported and skipped rather than keeping the agent down.
    pub fn open(dir: &Path) -> io::Result<Self> {
        let presets_dir = dir.join("presets");
        std::fs::create_dir_all(&presets_dir)?;
        let mut presets = BTreeMap::new();
        for entry in std::fs::read_dir(&presets_dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            match read::<StoredPreset>(&path) {
                Ok(preset) => {
                    presets.insert(preset.id, preset);
                }
                Err(error) => eprintln!("Skipping {}: {error}", path.display()),
            }
        }
        let active_path = dir.join("active.json");
        let active = match read::<StoredPreset>(&active_path) {
            Ok(active) => Some(active),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                eprintln!("Skipping {}: {error}", active_path.display());
                None
            }
        };
        Ok(Self {
            dir: dir.to_owned(),
            presets,
            active,
        })
    }

    pub fn list(&self) -> impl Iterator<Item = &StoredPreset> {
        self.presets.values()
    }

    pub fn get(&self, id: PresetId) -> Option<&StoredPreset> {
        self.presets.get(&id)
    }

    pub fn active(&self) -> Option<&StoredPreset> {
        self.active.as_ref()
    }

    /// Creates or replaces a preset, if it is as `expect` says.
    pub fn save(
        &mut self,
        id: PresetId,
        preset: Preset,
        expect: Option<Expect>,
        by: &str,
    ) -> Result<StoredPreset, SaveError> {
        let current = self.presets.get(&id).map(|stored| stored.revision);
        match (expect, current) {
            (None, _) => return Err(SaveError::Unconditional),
            (Some(Expect::Nothing), None) => {}
            (Some(Expect::Revision(revision)), Some(current)) if revision == current => {}
            _ => return Err(SaveError::Conflict(current)),
        }
        let stored = StoredPreset {
            id,
            revision: current.unwrap_or(0) + 1,
            updated_by: by.to_owned(),
            updated_at: now(),
            preset,
        };
        write_json(&self.preset_path(id), &stored).map_err(SaveError::Io)?;
        self.presets.insert(id, stored.clone());
        Ok(stored)
    }

    /// Whether there was one to delete.
    pub fn delete(&mut self, id: PresetId) -> io::Result<bool> {
        if !self.presets.contains_key(&id) {
            return Ok(false);
        }
        std::fs::remove_file(self.preset_path(id))?;
        self.presets.remove(&id);
        Ok(true)
    }

    pub fn set_active(&mut self, preset: StoredPreset) -> io::Result<()> {
        write_json(&self.dir.join("active.json"), &preset)?;
        self.active = Some(preset);
        Ok(())
    }

    fn preset_path(&self, id: PresetId) -> PathBuf {
        self.dir.join("presets").join(format!("{id}.json"))
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// A name next to `path` that no other write uses at the same time.
pub fn temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// Writes `bytes` so a reader sees either the old file or the whole new one,
/// even if the agent dies halfway.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temp_path(path);
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
