//! Locating the game, its per-user data folder, and the compatibility layer
//! it runs under.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const STEAM_APP_ID: u32 = 244210;
const GAME_DIR: &str = "assettocorsa";
const DOCUMENTS_DIR: &str = "Assetto Corsa";

#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    /// The game folder, containing `acs.exe` and `content/`.
    pub root: PathBuf,
    /// `Documents/Assetto Corsa`, where the game reads its session and
    /// settings from. It may not exist until the game has run once.
    pub documents: PathBuf,
    pub runtime: Runtime,
}

/// How the Windows-only game executable is started on this machine.
#[derive(Debug, Clone, PartialEq)]
pub enum Runtime {
    /// Windows: the executable runs directly.
    Native,
    /// Linux: the executable runs inside Steam's Proton prefix.
    Proton {
        steam_root: PathBuf,
        /// `steamapps/compatdata/244210`
        compat_data: PathBuf,
        /// The Proton build the prefix was last used with, if it has been
        /// created yet.
        proton: Option<PathBuf>,
    },
}

impl Install {
    /// Finds the game through `MURETTO_AC_ROOT`, or by searching every Steam
    /// library on this machine.
    pub fn discover() -> Result<Self> {
        if let Some(root) = std::env::var_os("MURETTO_AC_ROOT").map(PathBuf::from)
            && root.join("content").is_dir()
        {
            return Ok(Self::at(root, steam_roots().into_iter().next()));
        }
        for steam_root in steam_roots() {
            for library in steam_libraries(&steam_root) {
                let root = library.join("steamapps/common").join(GAME_DIR);
                if root.join("content").is_dir() {
                    return Ok(Self::at(root, Some(steam_root)));
                }
            }
        }
        Err(Error::InstallNotFound)
    }

    /// Describes an install at a known location.
    pub fn at(root: PathBuf, steam_root: Option<PathBuf>) -> Self {
        if cfg!(windows) {
            let documents = dirs::document_dir()
                .unwrap_or_else(|| root.clone())
                .join(DOCUMENTS_DIR);
            return Self {
                root,
                documents,
                runtime: Runtime::Native,
            };
        }

        // <library>/steamapps/common/assettocorsa -> <library>/steamapps
        let steamapps = root
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.clone());
        let compat_data = steamapps.join("compatdata").join(STEAM_APP_ID.to_string());
        let documents = compat_data
            .join("pfx/drive_c/users/steamuser/Documents")
            .join(DOCUMENTS_DIR);
        let proton = std::fs::read_to_string(compat_data.join("config_info"))
            .ok()
            .and_then(|info| proton_from_config_info(&info));
        // <steam_root>/steamapps is the default library; fall back to it
        // when the caller could not tell us where the client lives.
        let steam_root = steam_root.unwrap_or_else(|| {
            steamapps
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        });
        Self {
            root,
            documents,
            runtime: Runtime::Proton {
                steam_root,
                compat_data,
                proton,
            },
        }
    }

    pub fn cars_dir(&self) -> PathBuf {
        self.root.join("content/cars")
    }

    pub fn tracks_dir(&self) -> PathBuf {
        self.root.join("content/tracks")
    }

    pub fn weather_dir(&self) -> PathBuf {
        self.root.join("content/weather")
    }

    /// Where the game reads `race.ini`, `assists.ini` and the other settings.
    pub fn cfg_dir(&self) -> PathBuf {
        self.documents.join("cfg")
    }

    /// Where the game and Custom Shaders Patch write their logs and crash
    /// reports.
    pub fn logs_dir(&self) -> PathBuf {
        self.documents.join("logs")
    }

    pub fn executable(&self) -> PathBuf {
        self.root.join("acs.exe")
    }
}

/// Places the Steam client is installed, most common first.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(dir) = std::env::var_os(var) {
                roots.push(PathBuf::from(dir).join("Steam"));
            }
        }
    } else if let Some(home) = dirs::home_dir() {
        roots.extend([
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".steam/root"),
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            home.join("snap/steam/common/.local/share/Steam"),
            home.join("Library/Application Support/Steam"),
        ]);
    }
    let mut seen = Vec::new();
    roots.retain(|root| {
        let Ok(real) = root.canonicalize() else {
            return false;
        };
        let new = !seen.contains(&real);
        seen.push(real);
        new
    });
    roots
}

/// Every library folder the client at `steam_root` knows about, including
/// its own.
pub fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![steam_root.to_path_buf()];
    for file in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
        if let Ok(text) = std::fs::read_to_string(steam_root.join(file)) {
            for path in library_paths(&text) {
                if !libraries.contains(&path) {
                    libraries.push(path);
                }
            }
        }
    }
    libraries
}

/// The `"path"` entries of a `libraryfolders.vdf`.
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            (quoted.next()? == "path").then(|| quoted.next())?
        })
        .map(|path| PathBuf::from(path.replace("\\\\", "\\")))
        .collect()
}

/// `config_info` lists paths inside the Proton build that created the
/// prefix; the build's root is whatever precedes `files/` or `dist/`.
fn proton_from_config_info(info: &str) -> Option<PathBuf> {
    info.lines().find_map(|line| {
        let end = line.find("/files/").or_else(|| line.find("/dist/"))?;
        let dir = PathBuf::from(&line[..end]);
        dir.join("proton").is_file().then_some(dir)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_are_read_from_vdf() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.local/share/Steam"
		"label"		""
		"apps" { "228980"		"513685274" }
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            library_paths(vdf),
            [
                PathBuf::from("/home/u/.local/share/Steam"),
                PathBuf::from("D:\\SteamLibrary")
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn proton_prefix_is_derived_from_the_game_folder() {
        let install = Install::at(
            PathBuf::from("/lib/steamapps/common/assettocorsa"),
            Some(PathBuf::from("/steam")),
        );
        assert_eq!(
            install.documents,
            PathBuf::from(
                "/lib/steamapps/compatdata/244210/pfx/drive_c/users/steamuser/Documents/Assetto Corsa"
            )
        );
        assert_eq!(install.cfg_dir(), install.documents.join("cfg"));
        assert!(matches!(
            install.runtime,
            Runtime::Proton { ref steam_root, proton: None, .. } if steam_root == Path::new("/steam")
        ));
    }
}
