//! What Muretto knows about well-known mods: where the player gets each one,
//! how to recognise its archive, and what installing it involves. One entry
//! per mod, maintained by hand.
//!
//! These mods are laid out like the game folder and are downloaded by the
//! player, usually from behind a login. Muretto opens the page and takes the
//! archive from there.

use std::path::Path;

use super::{Progress, Receipt, Replaced, Staged, csp, uninstall, weather_fx};
use crate::{Error, Install, Result, ini::Ini, lenient};

pub struct Recipe {
    /// Names the receipt; never change it once released.
    pub id: &'static str,
    pub name: &'static str,
    pub author: &'static str,
    pub summary: &'static str,
    /// Where the player downloads it.
    pub page: &'static str,
    /// What to call that place: "Patreon", "GitHub".
    pub page_name: &'static str,
    /// Top-level folders of the archive that belong in the game folder.
    pub folders: &'static [&'static str],
    /// A file every archive of this mod has. Tells its archive from any
    /// other, and an installed copy from none.
    pub marker: &'static str,
    /// (file, section, key) holding its version.
    pub version: (&'static str, &'static str, &'static str),
    /// The oldest Custom Shaders Patch it runs on, if it needs the patch.
    pub needs_patch: Option<PatchNeed>,
    pub weather: Option<Weather>,
}

/// A Custom Shaders Patch build, by its build number and the version the
/// player knows it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchNeed {
    pub build: u32,
    pub version: &'static str,
}

/// What a mod adds to Weather FX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    /// Folder names of its weather styles, the one to offer first.
    pub styles: &'static [&'static str],
    /// The controller its styles go with.
    pub controller: &'static str,
}

pub const PURE: Recipe = Recipe {
    id: "pure",
    name: "Pure",
    author: "Peter Boese",
    summary: "A weather style for Weather FX with its own sky, clouds, lighting and post-processing filters.",
    page: "https://www.patreon.com/peterboese",
    page_name: "Patreon",
    // Checked against Pure 3.21. Beside these folders its archive holds
    // PDFs, a licence and Windows uninstall scripts, none for the game
    // folder. `content/weather` adds Sol's weather presets, which Pure uses.
    folders: &["apps", "content", "extension", "system"],
    marker: "extension/weather/pure/weather.lua",
    version: ("extension/weather/pure/manifest.ini", "ABOUT", "VERSION"),
    // Pure's own check, in `weather.lua`.
    needs_patch: Some(PatchNeed {
        build: 2639,
        version: "0.2.0",
    }),
    weather: Some(Weather {
        // "Pure Gamma", and "Pure LCS" for linear colour space shaders.
        styles: &["pure", "Pure LCS"],
        // "pureCtrl static" is also shipped; the dynamic one is Pure's default.
        controller: "pureCtrl",
    }),
};

pub const ALL: &[&Recipe] = &[&PURE];

/// A recipe's mod as found in the game folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub version: Option<String>,
    /// Installed by Muretto, which therefore knows every file to remove.
    pub managed: bool,
}

impl Recipe {
    pub fn installed(&self, install: &Install) -> Option<Installed> {
        if !install.root.join(self.marker).is_file() {
            return None;
        }
        let (file, ..) = self.version;
        Some(Installed {
            version: std::fs::read(install.root.join(file))
                .ok()
                .and_then(|bytes| self.version_in(&bytes)),
            managed: Receipt::load(&install.root, self.id).is_some(),
        })
    }

    /// Installs the mod from an archive the player downloaded, over any
    /// earlier version. Game files it overwrites are set aside.
    pub fn install_from_file(
        &self,
        install: &Install,
        archive: &Path,
        progress: &Progress,
    ) -> Result<Installed> {
        let belongs = |path: &str| {
            path.split_once('/')
                .is_some_and(|(folder, rest)| self.folders.contains(&folder) && !rest.is_empty())
        };
        let not_this = || {
            Error::Archive(format!(
                "This archive is not {}: it has no {}.",
                self.name, self.marker
            ))
        };
        let staged = Staged::unpack(&install.root, self.id, archive, belongs, progress).map_err(
            |error| match error {
                // Nothing for the game folder at all is the same finding.
                Error::Archive(message) if message.starts_with("Nothing in this archive") => {
                    not_this()
                }
                other => other,
            },
        )?;
        if !staged.contains(self.marker) {
            return Err(not_this());
        }
        let version = staged
            .read(self.version.0)
            .and_then(|bytes| self.version_in(&bytes))
            .unwrap_or_default();
        staged.install(self.id, &version, Replaced::Kept, progress)?;
        self.installed(install)
            .ok_or_else(|| Error::Archive(format!("{} did not install correctly.", self.name)))
    }

    /// Removes a copy Muretto installed and puts back what it overwrote.
    /// A copy installed by other means is refused: its files cannot be told
    /// from the game's.
    pub fn remove(&self, install: &Install) -> Result<()> {
        let was_in_use = self.in_use(install);
        if uninstall(&install.root, self.id)? {
            // Do not leave Weather FX pointing at a style that is gone.
            if was_in_use {
                weather_fx::use_style(install, weather_fx::BUILT_IN, None)?;
            }
            return Ok(());
        }
        Err(Error::Archive(format!(
            "{} was not installed by Muretto, so Muretto does not know which files are its own.",
            self.name
        )))
    }

    /// Why the installed patch cannot run this mod, if it cannot. A patch
    /// whose build is unknown is given the benefit of the doubt.
    pub fn patch_shortfall(&self, patch: Option<&csp::Installed>) -> Option<String> {
        let need = self.needs_patch?;
        let Some(patch) = patch else {
            return Some("Needs Custom Shaders Patch.".to_owned());
        };
        let build = patch
            .build
            .as_deref()
            .and_then(|b| b.trim().parse::<u32>().ok());
        match build {
            Some(build) if build < need.build => Some(format!(
                "Needs Custom Shaders Patch {} or newer.",
                need.version
            )),
            _ => None,
        }
    }

    /// Whether `style` is one of this mod's weather styles.
    pub fn has_style(&self, style: &str) -> bool {
        self.weather
            .is_some_and(|weather| weather.styles.contains(&style))
    }

    /// Whether one of this mod's weather styles is the one in use.
    pub fn in_use(&self, install: &Install) -> bool {
        let settings = weather_fx::settings(install);
        settings.enabled && self.has_style(&settings.style)
    }

    /// Makes this mod's first weather style the one in use.
    pub fn use_now(&self, install: &Install) -> Result<()> {
        match self
            .weather
            .and_then(|weather| weather.styles.first().copied())
        {
            Some(style) => self.use_style(install, style),
            None => Ok(()),
        }
    }

    /// Makes `style`, one of this mod's, the one in use, with its controller.
    pub fn use_style(&self, install: &Install, style: &str) -> Result<()> {
        let controller = self.weather.map(|weather| weather.controller);
        weather_fx::use_style(install, style, controller)
    }

    fn version_in(&self, file: &[u8]) -> Option<String> {
        let (_, section, key) = self.version;
        Ini::parse(&lenient::decode(file))
            .get(section, key)
            .filter(|version| !version.is_empty())
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{temp_dir, zip_of};
    use super::*;

    const MANIFEST: &str =
        "[ABOUT]\r\nNAME = Pure Gamma\r\nAUTHOR = Peter Boese\r\nVERSION = 3.21\r\n";

    fn game(name: &str) -> (std::path::PathBuf, Install) {
        let dir = temp_dir(name);
        let install = Install::at(dir.join("steamapps/common/assettocorsa"), None);
        std::fs::create_dir_all(install.root.join("system/cfg/ppfilters")).unwrap();
        std::fs::write(
            install.root.join("system/cfg/ppfilters/default.ini"),
            "stock",
        )
        .unwrap();
        (dir, install)
    }

    #[test]
    fn pure_is_installed_from_a_wrapped_archive_used_and_removed() {
        let (dir, install) = game("pure");
        assert_eq!(PURE.installed(&install), None);
        // Laid out like Pure 3.21's archive: documents beside a wrapping
        // folder that also holds files not meant for the game folder.
        zip_of(
            &dir.join("Pure 3.21 Highres.zip"),
            &[
                ("HDR - Tutorial.pdf", "not for the game folder"),
                ("_LICENSE and COPYRIGHT.txt", "not for the game folder"),
                ("Pure 3.21/extension/weather/pure/weather.lua", "script"),
                ("Pure 3.21/extension/weather/pure/manifest.ini", MANIFEST),
                ("Pure 3.21/extension/weather/Pure LCS/weather.lua", "script"),
                (
                    "Pure 3.21/extension/weather-controllers/pureCtrl/controller.lua",
                    "ctrl",
                ),
                (
                    "Pure 3.21/extension/config-ext/Pure/Skydome_sets/default_16k/sky.dds",
                    "sky",
                ),
                ("Pure 3.21/apps/lua/PureConfig/manifest.ini", "app"),
                (
                    "Pure 3.21/content/weather/sol_01_clear/weather.ini",
                    "preset",
                ),
                ("Pure 3.21/system/cfg/ppfilters/pure.ini", "filter"),
                ("Pure 3.21/system/cfg/ppfilters/default.ini", "pure's"),
                ("Pure 3.21/description.jsgme", "not for the game folder"),
                ("Pure 3.21/uninstall_pure.bat", "not for the game folder"),
            ],
        );

        let installed = PURE
            .install_from_file(
                &install,
                &dir.join("Pure 3.21 Highres.zip"),
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(
            installed,
            Installed {
                version: Some("3.21".to_owned()),
                managed: true,
            }
        );
        for file in [
            "apps/lua/PureConfig/manifest.ini",
            "content/weather/sol_01_clear/weather.ini",
            "extension/config-ext/Pure/Skydome_sets/default_16k/sky.dds",
            "system/cfg/ppfilters/pure.ini",
        ] {
            assert!(install.root.join(file).is_file(), "{file}");
        }
        for file in [
            "HDR - Tutorial.pdf",
            "description.jsgme",
            "uninstall_pure.bat",
        ] {
            assert!(!install.root.join(file).exists(), "{file}");
        }

        assert!(!PURE.in_use(&install));
        PURE.use_now(&install).unwrap();
        assert!(PURE.in_use(&install));
        assert_eq!(weather_fx::settings(&install).style, "pure");
        assert_eq!(weather_fx::settings(&install).controller, "pureCtrl");
        // The linear colour space variant counts as Pure too.
        PURE.use_style(&install, "Pure LCS").unwrap();
        assert!(PURE.in_use(&install));

        PURE.remove(&install).unwrap();
        assert_eq!(PURE.installed(&install), None);
        assert_eq!(weather_fx::settings(&install).style, weather_fx::BUILT_IN);
        assert!(!install.root.join("apps").exists() && !install.root.join("extension").exists());
        let filter = install.root.join("system/cfg/ppfilters/default.ini");
        assert_eq!(std::fs::read_to_string(filter).unwrap(), "stock");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn other_archives_are_refused_and_change_nothing() {
        let (dir, install) = game("pure-wrong");
        // A car mod has the right folders but is not Pure.
        zip_of(
            &dir.join("car.zip"),
            &[("content/cars/some_car/ui/ui_car.json", "{}")],
        );
        zip_of(&dir.join("photos.zip"), &[("holiday/beach.jpg", "")]);
        for archive in ["car.zip", "photos.zip"] {
            let result = PURE.install_from_file(&install, &dir.join(archive), &Progress::new());
            assert!(
                matches!(&result, Err(Error::Archive(m)) if m.starts_with("This archive is not Pure")),
                "{archive}: {result:?}"
            );
        }
        assert!(!install.root.join("content").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_copy_installed_by_other_means_is_recognised_but_not_removed() {
        let (dir, install) = game("pure-foreign");
        let script = install.root.join(PURE.marker);
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "script").unwrap();
        assert_eq!(
            PURE.installed(&install),
            Some(Installed {
                version: None,
                managed: false,
            })
        );
        assert!(PURE.remove(&install).is_err());
        assert!(script.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pure_names_the_patch_it_needs() {
        let patch = |build: Option<&str>| csp::Installed {
            version: Some("0.2.x".to_owned()),
            build: build.map(str::to_owned),
            managed: false,
        };
        let needs = |patch: Option<csp::Installed>| PURE.patch_shortfall(patch.as_ref());
        assert_eq!(needs(None).as_deref(), Some("Needs Custom Shaders Patch."));
        assert_eq!(
            needs(Some(patch(Some("2500")))).as_deref(),
            Some("Needs Custom Shaders Patch 0.2.0 or newer.")
        );
        assert_eq!(needs(Some(patch(Some("2639")))), None);
        assert_eq!(needs(Some(patch(None))), None);
    }
}
