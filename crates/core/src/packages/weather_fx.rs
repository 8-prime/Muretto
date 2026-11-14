//! Weather FX, the part of Custom Shaders Patch that draws sky, light and
//! clouds. Its look comes from a *weather style*: a script in
//! `extension/weather/<id>`. The patch ships one; mods such as Pure add more.
//!
//! The patch's defaults live in the game folder. What the player changes is
//! written to `Documents/Assetto Corsa/cfg/extension`, which is the only
//! place Muretto writes.

use std::path::PathBuf;

use crate::{Install, Result, ini::Ini, io_err, lenient};

const FILE: &str = "weather_fx.ini";
/// The style that ships with the patch.
pub const BUILT_IN: &str = "base";

/// An installed weather style.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    /// Folder name; what the setting refers to.
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// Id of the weather style in use.
    pub style: String,
    pub controller: String,
}

/// Every weather style in the game folder, by name.
pub fn styles(install: &Install) -> Vec<Style> {
    let dir = install.root.join("extension/weather");
    let mut styles: Vec<Style> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("weather.lua").is_file())
        .filter_map(|path| {
            let id = path.file_name()?.to_str()?.to_owned();
            let manifest = read_ini(path.join("manifest.ini"));
            let about = |key| {
                manifest
                    .get("ABOUT", key)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            Some(Style {
                name: about("NAME").unwrap_or_else(|| id.clone()),
                author: about("AUTHOR"),
                version: about("VERSION"),
                description: about("DESCRIPTION"),
                id,
            })
        })
        .collect();
    styles.sort_by_key(|style| style.name.to_lowercase());
    styles
}

/// The settings in force: the player's where set, the patch's defaults
/// otherwise.
pub fn settings(install: &Install) -> Settings {
    let defaults = read_ini(install.root.join("extension/config").join(FILE));
    let chosen = read_ini(user_file(install));
    let value = |key| {
        chosen
            .get("BASIC", key)
            .or_else(|| defaults.get("BASIC", key))
            .unwrap_or_default()
            .to_owned()
    };
    Settings {
        enabled: value("ENABLED") != "0",
        style: value("IMPLEMENTATION"),
        controller: value("CONTROLLER"),
    }
}

/// Switches Weather FX on with the given style. `controller` is set as well
/// when the game folder has a controller script of that name; otherwise the
/// current one stays.
pub fn use_style(install: &Install, style: &str, controller: Option<&str>) -> Result<()> {
    let path = user_file(install);
    let mut ini = read_ini(path.clone());
    ini.set("BASIC", "ENABLED", 1)
        .set("BASIC", "IMPLEMENTATION", style);
    let controllers = install.root.join("extension/weather-controllers");
    if let Some(controller) =
        controller.filter(|id| controllers.join(id).join("controller.lua").is_file())
    {
        ini.set("BASIC", "CONTROLLER", controller);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    std::fs::write(&path, ini.to_string().replace('\n', "\r\n")).map_err(io_err(&path))
}

fn user_file(install: &Install) -> PathBuf {
    install.cfg_dir().join("extension").join(FILE)
}

fn read_ini(path: PathBuf) -> Ini {
    std::fs::read(path)
        .map(|bytes| Ini::parse(&lenient::decode(&bytes)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::tests::temp_dir;
    use super::*;

    fn game(name: &str) -> Install {
        let install = Install::at(temp_dir(name).join("steamapps/common/assettocorsa"), None);
        let write = |path: &str, contents: &str| {
            let path = install.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        };
        write(
            "extension/config/weather_fx.ini",
            "[BASIC]\nENABLED=1 ; Active; 1 or 0\nCONTROLLER=sol ; Controller script\nIMPLEMENTATION=base ; Weather style\n[PERFORMANCE]\nHIGH_QUALITY_STARS=1\n",
        );
        write("extension/weather/base/weather.lua", "");
        write(
            "extension/weather/base/manifest.ini",
            "[ABOUT]\nNAME = Default\nAUTHOR = x4fab\nVERSION = 1.6\nDESCRIPTION = Simple built-in implementation.\n",
        );
        write("extension/weather/pure/weather.lua", "");
        write("extension/weather/broken/readme.txt", "no script here");
        write("extension/weather-controllers/pureCtrl/controller.lua", "");
        install
    }

    #[test]
    fn styles_are_folders_with_a_script() {
        let install = game("wfx-styles");
        let styles = styles(&install);
        let names: Vec<_> = styles
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        assert_eq!(names, [("base", "Default"), ("pure", "pure")]);
        assert_eq!(styles[0].version.as_deref(), Some("1.6"));
        assert_eq!(styles[1].version, None);
    }

    #[test]
    fn choosing_a_style_writes_only_the_players_file() {
        let install = game("wfx-use");
        assert_eq!(
            settings(&install),
            Settings {
                enabled: true,
                style: "base".to_owned(),
                controller: "sol".to_owned(),
            }
        );
        let defaults = std::fs::read(install.root.join("extension/config/weather_fx.ini")).unwrap();

        use_style(&install, "pure", Some("pureCtrl")).unwrap();
        assert_eq!(settings(&install).style, "pure");
        assert_eq!(settings(&install).controller, "pureCtrl");
        assert_eq!(
            std::fs::read(install.root.join("extension/config/weather_fx.ini")).unwrap(),
            defaults
        );

        // A controller that is not installed is not selected; other settings
        // the player made are kept.
        let mine = install.cfg_dir().join("extension/weather_fx.ini");
        std::fs::write(
            &mine,
            "[BASIC]\nENABLED=0\nCONTROLLER=sol\n[PERFORMANCE]\nHIGH_QUALITY_STARS=0\n",
        )
        .unwrap();
        use_style(&install, "base", Some("missing")).unwrap();
        let written = Ini::parse(&std::fs::read_to_string(&mine).unwrap());
        assert_eq!(written.get("BASIC", "ENABLED"), Some("1"));
        assert_eq!(written.get("BASIC", "CONTROLLER"), Some("sol"));
        assert_eq!(written.get("PERFORMANCE", "HIGH_QUALITY_STARS"), Some("0"));
    }
}
