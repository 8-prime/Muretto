//! Groups: friends who share a code see which server each of them is looking
//! at, so they can end up on the same one.
//!
//! A group is nothing but its code. Everyone who has it derives the same
//! room on the group server and the same key; the server sees the room, never
//! the code, and relays states sealed with the key that it cannot read. The
//! codes a player has joined are kept in `groups.json` in Muretto's settings
//! folder, so a group is set up once and rejoined whenever Muretto starts.

mod client;
pub mod relay;

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ring::{
    aead::{self, Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey},
    hkdf,
    rand::{SecureRandom as _, SystemRandom},
};
use serde_json::{Value, json};

pub use client::{Event, Group, Member, Status};
pub use relay::{Pipe, Relay, WebSocketRelay};

use crate::{Error, Result, io_err};

/// The group server Muretto uses unless told otherwise, by
/// `MURETTO_GROUP_SERVER` or the `server` entry of `groups.json`.
pub const DEFAULT_SERVER: Option<&str> = Some("wss://muretto.8-prime.dev");

/// Crockford's base32: no I, L, O or U, so a code read out loud or typed by
/// hand survives.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_BYTES: usize = 15;

/// A group's code: 120 random bits, written as six groups of four.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code([u8; CODE_BYTES]);

impl Code {
    pub fn random() -> Self {
        let mut bytes = [0; CODE_BYTES];
        SystemRandom::new()
            .fill(&mut bytes)
            .expect("the system has randomness");
        Code(bytes)
    }

    /// Reads a code as a person might paste or type it: any case, with or
    /// without dashes and spaces, O for 0 and I or L for 1.
    pub fn parse(text: &str) -> Option<Self> {
        let mut bits: u32 = 0;
        let mut held = 0;
        let mut bytes = Vec::with_capacity(CODE_BYTES);
        for c in text.chars() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            let c = match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            let value = ALPHABET.iter().position(|&a| a as char == c)? as u32;
            bits = (bits << 5) | value;
            held += 5;
            if held >= 8 {
                held -= 8;
                bytes.push((bits >> held) as u8);
                bits &= (1 << held) - 1;
            }
        }
        (held == 0).then_some(())?;
        bytes.try_into().ok().map(Code)
    }

    /// The room on the group server and the key states are sealed with.
    fn keys(&self) -> Keys {
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, b"muretto-group-v1").extract(&self.0);
        let mut room = [0u8; 32];
        prk.expand(&[b"room"], Len(32))
            .and_then(|okm| okm.fill(&mut room))
            .expect("32 bytes is a valid HKDF length");
        let key: UnboundKey = prk
            .expand(&[b"key"], &CHACHA20_POLY1305)
            .expect("a key length is a valid HKDF length")
            .into();
        Keys {
            room: room.iter().map(|b| format!("{b:02x}")).collect(),
            key: LessSafeKey::new(key),
        }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut text = String::with_capacity(29);
        let mut bits: u32 = 0;
        let mut held = 0;
        for &byte in &self.0 {
            bits = (bits << 8) | byte as u32;
            held += 8;
            while held >= 5 {
                held -= 5;
                if !text.is_empty() && text.len() % 5 == 4 {
                    text.push('-');
                }
                text.push(ALPHABET[((bits >> held) & 31) as usize] as char);
            }
            bits &= (1 << held) - 1;
        }
        f.write_str(&text)
    }
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

struct Keys {
    /// 64 hex characters.
    room: String,
    key: LessSafeKey,
}

impl Keys {
    /// A random nonce, then the sealed text, as base64. The room is bound in,
    /// so a state cannot be moved to another group.
    fn seal(&self, plain: &[u8]) -> String {
        let mut nonce = [0u8; aead::NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .expect("the system has randomness");
        let mut sealed = plain.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(self.room.as_bytes()),
                &mut sealed,
            )
            .expect("a small state seals");
        let mut packet = nonce.to_vec();
        packet.extend(sealed);
        BASE64.encode(packet)
    }

    fn open(&self, text: &str) -> Option<Vec<u8>> {
        let mut packet = BASE64.decode(text).ok()?;
        if packet.len() < aead::NONCE_LEN + CHACHA20_POLY1305.tag_len() {
            return None;
        }
        let mut sealed = packet.split_off(aead::NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(&packet).ok()?;
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(self.room.as_bytes()), &mut sealed)
            .ok()?;
        Some(plain.to_vec())
    }
}

/// A server, as one member points the others at it: only where it is. Its
/// name and the rest come from the receiver's own lobby list, or from asking
/// the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewed {
    pub ip: String,
    /// Where the server answers questions about itself; with `ip`, what
    /// `online::Server::address` gives.
    pub http_port: u16,
}

impl Viewed {
    pub fn address(&self) -> String {
        format!("{}:{}", self.ip, self.http_port)
    }
}

/// What a member tells the rest of the group.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Presence {
    /// Random per Muretto install; tells a member's reconnection from a
    /// second member.
    pub device: String,
    /// The member's Steam name: the one bit of free text sent.
    pub name: String,
    /// The server the member has open on the Online screen.
    pub viewing: Option<Viewed>,
}

/// The most bytes each part of a presence may take; anything past it is
/// cut. Together they keep a sealed presence under the server's limit.
const MAX_DEVICE: usize = 32;
const MAX_NAME: usize = 64;
/// An IPv6 address is at most 45 characters; a host name could be longer.
const MAX_HOST: usize = 64;

/// `text` without control characters, cut to at most `bytes` bytes on a
/// character boundary.
fn cut(text: &str, bytes: usize) -> String {
    let mut kept = String::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if kept.len() + c.len_utf8() > bytes {
            break;
        }
        kept.push(c);
    }
    kept
}

/// What an id or an address can hold: letters, digits, `.`, `:` and `-`.
fn plain(text: &str, bytes: usize) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-'))
        .take(bytes)
        .collect()
}

impl Presence {
    fn to_bytes(&self) -> Vec<u8> {
        json!({
            "v": 1,
            "device": plain(&self.device, MAX_DEVICE),
            "name": cut(&self.name, MAX_NAME),
            "viewing": self.viewing.as_ref().map(|viewed| json!({
                "ip": plain(&viewed.ip, MAX_HOST),
                "port": viewed.http_port,
            })),
        })
        .to_string()
        .into_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let viewing = match value.get("viewing") {
            Some(Value::Object(_)) => {
                let viewing = &value["viewing"];
                Some(Viewed {
                    ip: plain(viewing.get("ip")?.as_str()?, MAX_HOST),
                    http_port: viewing
                        .get("port")
                        .and_then(Value::as_u64)
                        .and_then(|port| u16::try_from(port).ok())?,
                })
            }
            _ => None,
        };
        Some(Presence {
            device: plain(value.get("device")?.as_str()?, MAX_DEVICE),
            name: cut(
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                MAX_NAME,
            ),
            viewing,
        })
    }
}

/// A group as kept between runs.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptGroup {
    pub code: Code,
    /// Names of the members last seen, to tell groups apart while nobody
    /// else is online.
    pub known: Vec<String>,
    /// Seconds since 1970 when it was joined.
    pub added: u64,
}

/// The groups the player is in, and what to call this install in them.
#[derive(Debug, Clone)]
pub struct Groups {
    path: Option<PathBuf>,
    /// Random per install; see [`Presence::device`].
    pub device: String,
    /// The group server, if `groups.json` names one.
    pub server: Option<String>,
    pub groups: Vec<KeptGroup>,
}

impl Groups {
    /// `<settings folder>/muretto/groups.json`
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("muretto").join("groups.json"))
    }

    /// Reads what was kept at `path`; a missing or damaged file is no
    /// groups. A new install gets its device id here, kept on the first save.
    pub fn load(path: Option<PathBuf>) -> Self {
        let value: Option<Value> = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        let value = value.unwrap_or(Value::Null);
        let groups = value
            .get("groups")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|group| {
                        Some(KeptGroup {
                            code: Code::parse(group.get("code")?.as_str()?)?,
                            known: group
                                .get("known")
                                .and_then(Value::as_array)
                                .map(|names| {
                                    names
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .map(str::to_owned)
                                        .collect()
                                })
                                .unwrap_or_default(),
                            added: group.get("added").and_then(Value::as_u64).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Groups {
            device: value
                .get("device")
                .and_then(Value::as_str)
                .filter(|device| !device.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(random_device),
            server: value
                .get("server")
                .and_then(Value::as_str)
                .filter(|server| !server.trim().is_empty())
                .map(str::to_owned),
            groups,
            path,
        }
    }

    /// The group server to use: `MURETTO_GROUP_SERVER`, then the file's
    /// `server`, then [`DEFAULT_SERVER`].
    pub fn server_url(&self) -> Option<String> {
        std::env::var("MURETTO_GROUP_SERVER")
            .ok()
            .filter(|server| !server.trim().is_empty())
            .or_else(|| self.server.clone())
            .or_else(|| DEFAULT_SERVER.map(str::to_owned))
    }

    /// Keeps `code`; joining a group already kept changes nothing.
    pub fn add(&mut self, code: Code) -> Result<()> {
        if self.groups.iter().any(|group| group.code == code) {
            return Ok(());
        }
        self.groups.push(KeptGroup {
            code,
            known: Vec::new(),
            added: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        });
        self.save()
    }

    pub fn remove(&mut self, code: &Code) -> Result<()> {
        self.groups.retain(|group| &group.code != code);
        self.save()
    }

    /// Remembers who was seen in the group; saves only when that changed.
    pub fn saw(&mut self, code: &Code, names: &[String]) -> Result<()> {
        let Some(group) = self.groups.iter_mut().find(|group| &group.code == code) else {
            return Ok(());
        };
        let mut changed = false;
        for name in names.iter().filter(|name| !name.trim().is_empty()) {
            if !group.known.contains(name) {
                group.known.push(name.clone());
                changed = true;
            }
        }
        if changed { self.save() } else { Ok(()) }
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let mut value = json!({
            "device": self.device,
            "groups": self.groups.iter().map(|group| json!({
                "code": group.code.to_string(),
                "known": group.known,
                "added": group.added,
            })).collect::<Vec<_>>(),
        });
        if let Some(server) = &self.server {
            value["server"] = json!(server);
        }
        std::fs::write(path, format!("{value:#}\n")).map_err(io_err(path))
    }
}

fn random_device() -> String {
    let mut bytes = [0u8; 8];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system has randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn network(text: impl Into<String>) -> Error {
    Error::Network(text.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_read_back_however_they_are_typed() {
        for _ in 0..50 {
            let code = Code::random();
            let text = code.to_string();
            assert_eq!(text.len(), 29, "{text}");
            assert_eq!(text.matches('-').count(), 5, "{text}");
            assert_eq!(Code::parse(&text), Some(code.clone()));
            assert_eq!(Code::parse(&text.to_lowercase()), Some(code.clone()));
            assert_eq!(Code::parse(&text.replace('-', " ")), Some(code.clone()));
            assert_eq!(Code::parse(&format!("  {text}\n")), Some(code.clone()));
            assert_eq!(Code::parse(&text.replace('-', "")), Some(code));
        }
        let code = Code::parse("0000-1111-2222-3333-4444-5555").unwrap();
        assert_eq!(
            Code::parse("oooo-iiii-2222-3333-4444-5555"),
            Some(code.clone())
        );
        assert_eq!(Code::parse("OOOO-LLLL-2222-3333-4444-5555"), Some(code));
        assert_eq!(
            Code::parse("0000-1111-2222-3333-4444-555"),
            None,
            "too short"
        );
        assert_eq!(
            Code::parse("0000-1111-2222-3333-4444-5555-6"),
            None,
            "too long"
        );
        assert_eq!(
            Code::parse("UUUU-1111-2222-3333-4444-5555"),
            None,
            "U is not used"
        );
        assert_eq!(Code::parse(""), None);
    }

    #[test]
    fn the_same_code_gives_the_same_room_and_key_and_another_code_cannot_read() {
        let code = Code::random();
        let (a, b) = (code.keys(), code.clone().keys());
        assert_eq!(a.room, b.room);
        assert!(muretto_group_protocol::valid_room(&a.room));
        let sealed = a.seal(b"hello");
        assert_ne!(a.seal(b"hello"), sealed, "every seal has its own nonce");
        assert_eq!(b.open(&sealed).as_deref(), Some(&b"hello"[..]));

        let other = Code::random().keys();
        assert_ne!(other.room, a.room);
        assert_eq!(other.open(&sealed), None);
        assert_eq!(a.open("not base64!"), None);
        assert_eq!(a.open(""), None);
        let mut tampered = BASE64.decode(&sealed).unwrap();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(a.open(&BASE64.encode(tampered)), None);
    }

    #[test]
    fn a_presence_survives_the_trip_and_fits_the_server_limit() {
        let presence = Presence {
            device: "abcd".into(),
            name: "Alex".into(),
            viewing: Some(Viewed {
                ip: "10.0.0.1".into(),
                http_port: 8081,
            }),
        };
        assert_eq!(Presence::from_bytes(&presence.to_bytes()), Some(presence));
        let idle = Presence {
            device: "abcd".into(),
            name: "Sam".into(),
            viewing: None,
        };
        assert_eq!(Presence::from_bytes(&idle.to_bytes()), Some(idle));
        assert_eq!(Presence::from_bytes(b"{}"), None);

        assert_eq!(cut("a\u{1F3CE}", 3), "a", "cut on a character boundary");
        assert_eq!(cut("Al\u{1}ex\n", 64), "Alex", "no control characters");
        assert_eq!(plain("10.0.0.1\"}", 64), "10.0.0.1");

        // The longest presence still seals under the server's limit, with
        // a name of the characters JSON escapes into two bytes each.
        let huge = Presence {
            device: "f".repeat(500),
            name: "\"".repeat(500),
            viewing: Some(Viewed {
                ip: "f".repeat(500),
                http_port: 65535,
            }),
        };
        let sealed = Code::random().keys().seal(&huge.to_bytes());
        assert!(
            sealed.len() <= muretto_group_protocol::MAX_STATE,
            "{}",
            sealed.len()
        );
    }

    #[test]
    fn groups_and_the_device_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("muretto-groups-{}", std::process::id()));
        let path = dir.join("groups.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut groups = Groups::load(Some(path.clone()));
        assert!(groups.groups.is_empty());
        let device = groups.device.clone();
        assert_eq!(device.len(), 16);
        let (one, two) = (Code::random(), Code::random());
        groups.add(one.clone()).unwrap();
        groups.add(two.clone()).unwrap();
        groups.add(one.clone()).unwrap();
        groups.saw(&one, &["Alex".into(), "".into()]).unwrap();

        let mut groups = Groups::load(Some(path.clone()));
        assert_eq!(groups.device, device);
        let codes: Vec<&Code> = groups.groups.iter().map(|group| &group.code).collect();
        assert_eq!(codes, [&one, &two]);
        assert_eq!(groups.groups[0].known, ["Alex"]);
        groups.remove(&one).unwrap();
        assert_eq!(Groups::load(Some(path)).groups.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
