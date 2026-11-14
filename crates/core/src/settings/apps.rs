//! Python apps: the in-game windows from `<game>/apps/python`, each switched
//! on or off in `python.ini` under its folder name in capitals.

use crate::{Install, Result};

use super::{Config, PYTHON};

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    /// The folder name.
    pub id: String,
    pub active: bool,
}

/// Every installed app, by name. The game's own `system` folder is not an
/// app.
pub fn apps(install: &Install, config: &Config) -> Vec<App> {
    let mut ids: Vec<String> = std::fs::read_dir(install.root.join("apps/python"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|id| !id.eq_ignore_ascii_case("system") && !id.starts_with('.'))
        .collect();
    ids.sort_by_key(|id| id.to_lowercase());
    ids.into_iter()
        .map(|id| App {
            active: config
                .flag(PYTHON, &id.to_uppercase(), "ACTIVE")
                .unwrap_or(false),
            id,
        })
        .collect()
}

pub fn set_active(config: &mut Config, id: &str, active: bool) -> Result<()> {
    config.set(
        PYTHON,
        &id.to_uppercase(),
        "ACTIVE",
        if active { "1" } else { "0" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_listed_with_their_state_and_switched() {
        let base = std::env::temp_dir().join(format!("muretto-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("steamapps/common/assettocorsa");
        for dir in [
            "apps/python/gMeter",
            "apps/python/Chat",
            "apps/python/system",
            "cfg",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("cfg/python.ini"), "[GMETER]\nACTIVE=1\n").unwrap();
        let install = Install::at(root, None);
        let mut config = Config::load(&install);

        let listed = apps(&install, &config);
        assert_eq!(
            listed,
            [
                App {
                    id: "Chat".into(),
                    active: false
                },
                App {
                    id: "gMeter".into(),
                    active: true
                },
            ]
        );
        set_active(&mut config, "Chat", true).unwrap();
        assert!(apps(&install, &config)[0].active);
        assert_eq!(config.get(PYTHON, "CHAT", "ACTIVE").as_deref(), Some("1"));
    }
}
