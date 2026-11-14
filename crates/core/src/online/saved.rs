//! Servers the player keeps: the favourites they starred and the ones they
//! joined lately. Kept in Muretto's own settings folder, not the game's.

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::Server;
use crate::{Result, io_err};

/// How many joined servers Recent remembers.
const RECENT: usize = 30;

/// A server as remembered: enough to find it again when the lobby no longer
/// lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    /// `ip:http-port`, as [`Server::address`] gives it.
    pub address: String,
    pub name: String,
    /// Seconds since 1970 when it was starred or last joined.
    pub at: u64,
}

impl Kept {
    fn of(server: &Server) -> Self {
        Kept {
            address: server.address(),
            name: server.name.clone(),
            at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        }
    }

    fn from_json(value: &Value) -> Option<Self> {
        Some(Kept {
            address: value.get("address")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str().unwrap_or_default().to_owned(),
            at: value.get("at").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    fn to_json(&self) -> Value {
        json!({ "address": self.address, "name": self.name, "at": self.at })
    }
}

#[derive(Debug, Default, Clone)]
pub struct Saved {
    /// Where they are kept; nowhere when there is no settings folder.
    path: Option<PathBuf>,
    pub favourites: Vec<Kept>,
    /// Most recently joined first.
    pub recent: Vec<Kept>,
}

impl Saved {
    /// `<settings folder>/muretto/servers.json`
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("muretto").join("servers.json"))
    }

    /// Reads what was kept at `path`; a missing or damaged file is an empty
    /// list.
    pub fn load(path: Option<PathBuf>) -> Self {
        let value: Option<Value> = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        let list = |key: &str| -> Vec<Kept> {
            value
                .as_ref()
                .and_then(|value| value.get(key))
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Kept::from_json).collect())
                .unwrap_or_default()
        };
        Saved {
            favourites: list("favourites"),
            recent: list("recent"),
            path,
        }
    }

    pub fn is_favourite(&self, address: &str) -> bool {
        self.favourites.iter().any(|kept| kept.address == address)
    }

    /// Stars the server, or takes the star away.
    pub fn toggle_favourite(&mut self, server: &Server) -> Result<()> {
        let address = server.address();
        if self.is_favourite(&address) {
            self.favourites.retain(|kept| kept.address != address);
        } else {
            self.favourites.push(Kept::of(server));
        }
        self.save()
    }

    /// Puts the server at the top of Recent.
    pub fn joined(&mut self, server: &Server) -> Result<()> {
        let kept = Kept::of(server);
        self.recent.retain(|other| other.address != kept.address);
        self.recent.insert(0, kept);
        self.recent.truncate(RECENT);
        // A favourite's name follows the server's.
        for favourite in &mut self.favourites {
            if favourite.address == server.address() {
                favourite.name = server.name.clone();
            }
        }
        self.save()
    }

    /// Every address kept, favourites first.
    pub fn addresses(&self) -> impl Iterator<Item = &Kept> {
        self.favourites.iter().chain(&self.recent)
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let value = json!({
            "favourites": self.favourites.iter().map(Kept::to_json).collect::<Vec<_>>(),
            "recent": self.recent.iter().map(Kept::to_json).collect::<Vec<_>>(),
        });
        std::fs::write(path, format!("{value:#}\n")).map_err(io_err(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::parse_lobby;

    #[test]
    fn favourites_and_recent_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("muretto-saved-{}", std::process::id()));
        let path = dir.join("servers.json");
        let _ = std::fs::remove_dir_all(&dir);
        let servers = parse_lobby(
            r#"[{"ip":"10.0.0.1","port":9600,"cport":8081,"name":"One","track":"park","cars":[]},
                {"ip":"10.0.0.2","port":9600,"cport":8081,"name":"Two","track":"park","cars":[]}]"#,
        )
        .unwrap();

        let mut saved = Saved::load(Some(path.clone()));
        assert!(saved.favourites.is_empty());
        saved.toggle_favourite(&servers[0]).unwrap();
        saved.joined(&servers[0]).unwrap();
        saved.joined(&servers[1]).unwrap();
        saved.joined(&servers[0]).unwrap();

        let saved = Saved::load(Some(path.clone()));
        assert!(saved.is_favourite("10.0.0.1:8081"));
        assert!(!saved.is_favourite("10.0.0.2:8081"));
        let recent: Vec<&str> = saved.recent.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(recent, ["One", "Two"], "most recent first, each once");

        let mut saved = saved;
        saved.toggle_favourite(&servers[0]).unwrap();
        assert!(!Saved::load(Some(path)).is_favourite("10.0.0.1:8081"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
