//! Muretto's own settings, as against the game's: `settings.json` in
//! Muretto's settings folder. Keys this version does not know are kept.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::{Result, io_err};

const HOSTING: &str = "hosting";

#[derive(Debug, Clone, Default)]
pub struct Prefs {
    path: Option<PathBuf>,
    values: Map<String, Value>,
}

impl Prefs {
    /// `<settings folder>/muretto/settings.json`
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("muretto").join("settings.json"))
    }

    /// Reads `path`; a missing or damaged file is every setting at its
    /// default.
    pub fn load(path: Option<PathBuf>) -> Self {
        let values = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| match value {
                Value::Object(values) => Some(values),
                _ => None,
            })
            .unwrap_or_default();
        Self { path, values }
    }

    /// Whether the Host section is shown: for players who run servers.
    pub fn hosting(&self) -> bool {
        self.values
            .get(HOSTING)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_hosting(&mut self, on: bool) -> Result<()> {
        self.values.insert(HOSTING.to_owned(), Value::Bool(on));
        self.save()
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let text = format!("{:#}\n", Value::Object(self.values.clone()));
        std::fs::write(path, text).map_err(io_err(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosting_is_off_until_turned_on_and_other_keys_survive() {
        let dir = std::env::temp_dir().join(format!("muretto-prefs-{}", std::process::id()));
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"later": 1}"#).unwrap();

        let mut prefs = Prefs::load(Some(path.clone()));
        assert!(!prefs.hosting());
        prefs.set_hosting(true).unwrap();
        let again = Prefs::load(Some(path.clone()));
        assert!(again.hosting());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"later\": 1"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
