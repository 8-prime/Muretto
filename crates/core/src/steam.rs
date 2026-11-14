//! The Steam account the game runs under. Online servers know players by
//! their 64-bit Steam id, and the lobby only answers requests that carry one.

use std::path::Path;

use crate::install::steam_roots;

#[derive(Debug, Clone, PartialEq)]
pub struct SteamUser {
    /// SteamID64, as servers expect it.
    pub id: String,
    /// The name friends see.
    pub name: String,
}

/// The account last signed in to the Steam client on this machine.
pub fn current_user() -> Option<SteamUser> {
    steam_roots()
        .iter()
        .find_map(|root| user_in(&root.join("config/loginusers.vdf")))
}

fn user_in(path: &Path) -> Option<SteamUser> {
    let text = std::fs::read_to_string(path).ok()?;
    most_recent(&parse_users(&text)).cloned()
}

struct Entry {
    user: SteamUser,
    most_recent: bool,
    timestamp: u64,
}

fn parse_users(vdf: &str) -> Vec<Entry> {
    let mut users = Vec::new();
    let mut current: Option<Entry> = None;
    for line in vdf.lines() {
        let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        match quoted.as_slice() {
            // A user's block opens with their id alone on a line.
            [id] if id.len() == 17 && id.bytes().all(|b| b.is_ascii_digit()) => {
                users.extend(current.take());
                current = Some(Entry {
                    user: SteamUser {
                        id: (*id).to_owned(),
                        name: String::new(),
                    },
                    most_recent: false,
                    timestamp: 0,
                });
            }
            [key, value] => {
                if let Some(entry) = &mut current {
                    match key.to_ascii_lowercase().as_str() {
                        "personaname" => entry.user.name = (*value).to_owned(),
                        "mostrecent" => entry.most_recent = *value == "1",
                        "timestamp" => entry.timestamp = value.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    users.extend(current);
    users
}

fn most_recent(users: &[Entry]) -> Option<&SteamUser> {
    users
        .iter()
        .max_by_key(|entry| (entry.most_recent, entry.timestamp))
        .map(|entry| &entry.user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_recent_account_is_chosen() {
        let vdf = r#""users"
{
	"76561197960287930"
	{
		"AccountName"		"old"
		"PersonaName"		"Old Account"
		"MostRecent"		"0"
		"Timestamp"		"1700000000"
	}
	"76561198000000001"
	{
		"AccountName"		"new"
		"PersonaName"		"L. Brandt"
		"MostRecent"		"1"
		"Timestamp"		"1600000000"
	}
}"#;
        let users = parse_users(vdf);
        assert_eq!(users.len(), 2);
        assert_eq!(
            most_recent(&users),
            Some(&SteamUser {
                id: "76561198000000001".to_owned(),
                name: "L. Brandt".to_owned(),
            })
        );
    }

    #[test]
    fn without_a_most_recent_flag_the_latest_sign_in_wins() {
        let vdf = r#""users"
{
	"76561197960287930"
	{
		"PersonaName"		"First"
		"timestamp"		"1"
	}
	"76561198000000001"
	{
		"PersonaName"		"Second"
		"timestamp"		"2"
	}
}"#;
        assert_eq!(most_recent(&parse_users(vdf)).unwrap().name, "Second");
    }
}
