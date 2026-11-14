//! The game's own settings: the INI files in `Documents/Assetto Corsa/cfg`
//! that the stock launcher edits. Files the player has never changed are
//! read from the game's defaults in `<game>/cfg`, and only written to the
//! settings folder when something in them changes.

pub mod apps;
pub mod controls;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{Install, Result, ini::Ini, io_err, launch, lenient};

pub const VIDEO: &str = "video.ini";
pub const AUDIO: &str = "audio.ini";
pub const CONTROLS: &str = "controls.ini";
pub const GAMEPLAY: &str = "gameplay.ini";
pub const PYTHON: &str = "python.ini";
pub const REPLAY: &str = "replay.ini";
pub const CAMERA: &str = "camera_onboard.ini";
pub const FF_POST: &str = "ff_post_process.ini";
pub const ASSISTS: &str = "assists.ini";

/// The files Muretto shows and edits.
const FILES: [&str; 9] = [
    VIDEO, AUDIO, CONTROLS, GAMEPLAY, PYTHON, REPLAY, CAMERA, FF_POST, ASSISTS,
];

/// Settings files as last read or written.
#[derive(Debug, Clone)]
pub struct Config {
    /// The player's settings folder.
    cfg: PathBuf,
    /// The game's defaults, copied into `cfg` on first start.
    defaults: PathBuf,
    files: HashMap<String, Ini>,
}

impl Config {
    pub fn load(install: &Install) -> Self {
        let mut config = Self {
            cfg: install.cfg_dir(),
            defaults: install.root.join("cfg"),
            files: HashMap::new(),
        };
        config.reload();
        config
    }

    pub fn dir(&self) -> &Path {
        &self.cfg
    }

    /// The file as the game would read it: the player's copy, or the
    /// default when there is none.
    pub fn file(&self, name: &str) -> &Ini {
        static EMPTY: Ini = Ini::new();
        self.files.get(name).unwrap_or(&EMPTY)
    }

    /// Whether the player has their own copy of `name`, rather than only
    /// the game's default.
    pub fn has_own(&self, name: &str) -> bool {
        self.cfg.join(name).is_file()
    }

    pub fn get(&self, file: &str, section: &str, key: &str) -> Option<String> {
        self.file(file).get(section, key).map(str::to_owned)
    }

    pub fn number(&self, file: &str, section: &str, key: &str) -> Option<f64> {
        self.file(file).get_number(section, key)
    }

    pub fn flag(&self, file: &str, section: &str, key: &str) -> Option<bool> {
        self.number(file, section, key).map(|n| n != 0.)
    }

    pub fn set(&mut self, file: &str, section: &str, key: &str, value: &str) -> Result<()> {
        self.edit(file, |ini| {
            ini.set(section, key, value);
        })
    }

    /// Changes a file and writes it straight away.
    pub fn edit(&mut self, file: &str, change: impl FnOnce(&mut Ini)) -> Result<()> {
        let mut ini = self.file(file).clone();
        change(&mut ini);
        self.write(file, &ini)?;
        self.files.insert(file.to_owned(), ini);
        Ok(())
    }

    /// Replaces a whole file.
    pub fn replace(&mut self, file: &str, ini: Ini) -> Result<()> {
        self.write(file, &ini)?;
        self.files.insert(file.to_owned(), ini);
        Ok(())
    }

    fn write(&self, file: &str, ini: &Ini) -> Result<()> {
        // The game will not start with half its settings missing, so a
        // first write brings the others along.
        launch::seed_defaults(&self.defaults, &self.cfg)?;
        launch::write(&self.cfg.join(file), &ini.to_string())
    }

    /// Puts `files` back to the game's defaults.
    pub fn restore_defaults(&mut self, files: &[&str]) -> Result<()> {
        for &file in files {
            let ini = read(&self.defaults.join(file)).unwrap_or_default();
            self.replace(file, ini)?;
        }
        Ok(())
    }

    /// Reads every file again, after something else may have changed them.
    pub fn reload(&mut self) {
        self.files = FILES
            .iter()
            .map(|&name| {
                let ini = read(&self.cfg.join(name))
                    .or_else(|| read(&self.defaults.join(name)))
                    .unwrap_or_default();
                (name.to_owned(), ini)
            })
            .collect();
    }

    /// The game's post-processing filters, by file name without `.ini`.
    pub fn pp_filters(install: &Install) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(install.root.join("system/cfg/ppfilters"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let stem = name.strip_suffix(".ini")?;
                Some(stem.to_owned())
            })
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names
    }

    /// Copies a force-feedback curve into the settings folder and turns it
    /// on. Returns the name it was saved under.
    pub fn import_ff_curve(&mut self, source: &Path) -> Result<String> {
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("custom.lut")
            .to_owned();
        let target = self.cfg.join(&name);
        launch::seed_defaults(&self.defaults, &self.cfg)?;
        if source != target {
            std::fs::copy(source, &target).map_err(io_err(&target))?;
        }
        self.edit(FF_POST, |ini| {
            ini.set("HEADER", "ENABLED", 1)
                .set("HEADER", "TYPE", "LUT")
                .set("LUT", "CURVE", &name);
        })?;
        Ok(name)
    }
}

fn read(path: &Path) -> Option<Ini> {
    let bytes = std::fs::read(path).ok()?;
    Some(Ini::parse(&lenient::decode(&bytes)))
}

/// `FPS_CAP_MS` holds the frame time; the launcher offers frame rates.
/// `None` when there is no limit.
pub fn fps_from_cap(milliseconds: f64) -> Option<u32> {
    (milliseconds > 0.).then(|| (1000. / milliseconds).round() as u32)
}

pub fn cap_from_fps(fps: Option<u32>) -> String {
    match fps {
        Some(fps) if fps > 0 => format!("{:.4}", 1000. / f64::from(fps)),
        _ => "0".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(name: &str) -> Install {
        let base =
            std::env::temp_dir().join(format!("muretto-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("steamapps/common/assettocorsa");
        std::fs::create_dir_all(root.join("cfg")).unwrap();
        std::fs::write(
            root.join("cfg/audio.ini"),
            "[LEVELS]\r\nMASTER=1 ; all of it\r\nENGINE=0.8\r\n",
        )
        .unwrap();
        std::fs::write(root.join("cfg/video.ini"), "[VIDEO]\r\nVSYNC=0\r\n").unwrap();
        Install::at(root, None)
    }

    #[test]
    fn defaults_are_read_until_the_first_change_then_all_are_seeded() {
        let install = install("seed");
        let mut config = Config::load(&install);
        assert_eq!(config.number(AUDIO, "LEVELS", "ENGINE"), Some(0.8));
        assert!(!config.has_own(AUDIO));
        assert!(!install.cfg_dir().exists());

        config.set(AUDIO, "LEVELS", "ENGINE", "0.5").unwrap();
        let written = std::fs::read_to_string(install.cfg_dir().join(AUDIO)).unwrap();
        assert_eq!(written, "[LEVELS]\r\nMASTER=1\r\nENGINE=0.5\r\n");
        // The other defaults came along, untouched.
        assert!(install.cfg_dir().join(VIDEO).is_file());

        let fresh = Config::load(&install);
        assert_eq!(fresh.number(AUDIO, "LEVELS", "ENGINE"), Some(0.5));
    }

    #[test]
    fn restoring_defaults_overwrites_the_players_copy() {
        let install = install("restore");
        let mut config = Config::load(&install);
        config.set(VIDEO, "VIDEO", "VSYNC", "1").unwrap();
        config.restore_defaults(&[VIDEO]).unwrap();
        assert_eq!(config.flag(VIDEO, "VIDEO", "VSYNC"), Some(false));
        config.reload();
        assert_eq!(config.flag(VIDEO, "VIDEO", "VSYNC"), Some(false));
    }

    #[test]
    fn frame_limits_are_stored_as_frame_times() {
        assert_eq!(cap_from_fps(Some(120)), "8.3333");
        assert_eq!(fps_from_cap(8.3333), Some(120));
        assert_eq!(fps_from_cap(16.666666), Some(60));
        assert_eq!(cap_from_fps(None), "0");
        assert_eq!(fps_from_cap(0.), None);
    }
}
