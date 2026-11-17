//! Server hosts: machines running Muretto's host agent next to
//! AssettoServer. The player keeps a list of them, each an address and the
//! token its owner handed out, in `hosts.json` in Muretto's settings folder.
//! [`Client`] speaks the agent's API, which `muretto-host-protocol`
//! describes; [`sync`] puts the files a preset needs on the host.

mod sync;

use std::{
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub use muretto_host_protocol as protocol;
use muretto_host_protocol::{
    ApiError, PATH_PREFIX, Status,
    content::{ContentPath, Installed, Manifest, SHA256_HEADER, UploadPlan},
    presets::{Activate, FieldError, Preset, PresetId, PresetSummary, StoredPreset},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
pub use sync::{LocalFile, Synced, local_files, sync};

use crate::{Result, io_err};

/// A host as the player keeps it.
#[derive(Clone, PartialEq, Eq)]
pub struct KeptHost {
    /// What the player calls it.
    pub name: String,
    /// Where the agent answers, as [`normalise_url`] wrote it.
    pub url: String,
    pub token: String,
    /// Seconds since 1970 when it was added.
    pub added: u64,
}

impl std::fmt::Debug for KeptHost {
    // The token stays out of logs and panics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeptHost")
            .field("name", &self.name)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl KeptHost {
    pub fn client(&self) -> Client {
        Client::new(&self.url, &self.token)
    }
}

/// The hosts the player has added.
#[derive(Debug, Clone, Default)]
pub struct Hosts {
    path: Option<PathBuf>,
    pub hosts: Vec<KeptHost>,
}

impl Hosts {
    /// `<settings folder>/muretto/hosts.json`
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("muretto").join("hosts.json"))
    }

    /// Reads what was kept at `path`; a missing or damaged file is no hosts.
    pub fn load(path: Option<PathBuf>) -> Self {
        let value: Option<Value> = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        let hosts = value
            .as_ref()
            .and_then(|value| value.get("hosts"))
            .and_then(Value::as_array)
            .map(|hosts| {
                hosts
                    .iter()
                    .filter_map(|host| {
                        let text = |key| host.get(key).and_then(Value::as_str);
                        Some(KeptHost {
                            url: normalise_url(text("url")?)?,
                            token: text("token")?.to_owned(),
                            name: text("name").unwrap_or_default().to_owned(),
                            added: host.get("added").and_then(Value::as_u64).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Hosts { path, hosts }
    }

    /// Keeps a host; one at the same address is replaced, so a new token
    /// can be put in by adding the host again.
    pub fn add(&mut self, name: &str, url: &str, token: &str) -> Result<&KeptHost> {
        let url = normalise_url(url)
            .ok_or_else(|| crate::Error::Network(format!("{url} is not the address of a host.")))?;
        let name = match name.trim() {
            "" => host_name(&url),
            name => name.to_owned(),
        };
        let kept = KeptHost {
            name,
            url,
            token: token.trim().to_owned(),
            added: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let at = match self.hosts.iter().position(|host| host.url == kept.url) {
            Some(at) => {
                self.hosts[at] = kept;
                at
            }
            None => {
                self.hosts.push(kept);
                self.hosts.len() - 1
            }
        };
        self.save()?;
        Ok(&self.hosts[at])
    }

    pub fn remove(&mut self, url: &str) -> Result<()> {
        self.hosts.retain(|host| host.url != url);
        self.save()
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let value = json!({
            "hosts": self.hosts.iter().map(|host| json!({
                "name": host.name,
                "url": host.url,
                "token": host.token,
                "added": host.added,
            })).collect::<Vec<_>>(),
        });
        let text = format!("{value:#}\n");
        // The tokens let anyone restart the server: only the player reads
        // the file.
        #[cfg(unix)]
        {
            use std::{io::Write as _, os::unix::fs::OpenOptionsExt as _};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
                .map_err(io_err(path))?;
            file.write_all(text.as_bytes()).map_err(io_err(path))
        }
        #[cfg(not(unix))]
        std::fs::write(path, text).map_err(io_err(path))
    }
}

/// An address as a player types it, made into the agent's base URL:
/// `https://` unless another scheme is given, no path or trailing slash.
/// `None` when nothing is left of it.
pub fn normalise_url(text: &str) -> Option<String> {
    let text = text.trim();
    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_owned(), text),
    };
    let rest = rest.trim_end_matches('/');
    if !matches!(scheme.as_str(), "http" | "https") {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    let bad = |c: char| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '@');
    if authority.is_empty() || authority.chars().any(bad) {
        return None;
    }
    let path = rest[authority.len()..].trim_end_matches('/');
    // The agent's prefix is added to every request; a pasted URL may
    // already carry it.
    let path = path.strip_suffix(PATH_PREFIX).unwrap_or(path);
    Some(format!("{scheme}://{authority}{path}"))
}

/// The host part of a URL, to name a host the player gave no name.
fn host_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', ':']).next().unwrap_or(rest).to_owned()
}

/// Why a request to a host came to nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum HostError {
    /// It could not be reached, or did not answer like an agent.
    Unreachable(String),
    /// The agent answered with a refusal.
    Refused {
        status: u16,
        message: String,
        fields: Vec<FieldError>,
    },
    Cancelled,
}

impl HostError {
    pub fn status(&self) -> Option<u16> {
        match self {
            HostError::Refused { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// What was wrong with each field of a refused preset.
    pub fn fields(&self) -> &[FieldError] {
        match self {
            HostError::Refused { fields, .. } => fields,
            _ => &[],
        }
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Unreachable(text) => f.write_str(text),
            HostError::Refused { message, .. } => f.write_str(message),
            HostError::Cancelled => f.write_str("Cancelled."),
        }
    }
}

impl std::error::Error for HostError {}

/// Conflict: a stale revision on save, or drivers on the server on activate.
pub const CONFLICT: u16 = 409;
/// The preset has mistakes, or the host lacks its content.
pub const UNPROCESSABLE: u16 = 422;

pub type HostResult<T> = std::result::Result<T, HostError>;

/// Speaks to one host's agent. Every call blocks; run them off the
/// interface thread.
#[derive(Clone)]
pub struct Client {
    /// The URL with the API prefix.
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(url: &str, token: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .user_agent(concat!("Muretto/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(10)))
            // Activating stops and starts the server before it answers.
            .timeout_recv_response(Some(Duration::from_secs(60)))
            // Refusals carry an explanation in their body.
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: format!("{}{PATH_PREFIX}", url.trim_end_matches('/')),
            token: token.to_owned(),
            agent,
        }
    }

    pub fn status(&self) -> HostResult<Status> {
        self.json(self.agent.get(self.url("/status")))
    }

    pub fn presets(&self) -> HostResult<Vec<PresetSummary>> {
        self.json(self.agent.get(self.url("/presets")))
    }

    pub fn preset(&self, id: PresetId) -> HostResult<StoredPreset> {
        self.json(self.agent.get(self.url(&format!("/presets/{id}"))))
    }

    /// Saves a new preset under `id`. Repeating it after a timeout finds the
    /// first copy and is refused, rather than making a second.
    pub fn create(&self, id: PresetId, preset: &Preset) -> HostResult<StoredPreset> {
        let request = self
            .agent
            .put(self.url(&format!("/presets/{id}")))
            .header("If-None-Match", "*");
        self.send_json(request, preset)
    }

    /// Replaces the preset at `revision`; [`CONFLICT`] when it was saved
    /// since.
    pub fn save(&self, id: PresetId, revision: u64, preset: &Preset) -> HostResult<StoredPreset> {
        let request = self
            .agent
            .put(self.url(&format!("/presets/{id}")))
            .header("If-Match", revision.to_string());
        self.send_json(request, preset)
    }

    pub fn delete(&self, id: PresetId) -> HostResult<()> {
        let request = self.agent.delete(self.url(&format!("/presets/{id}")));
        let response = request
            .header("Authorization", self.bearer())
            .call()
            .map_err(|e| self.unreachable(e))?;
        self.read(response).map(|_| ())
    }

    /// Writes the preset into the server's config and restarts it.
    /// [`CONFLICT`] while drivers are on, unless `force`; [`UNPROCESSABLE`]
    /// when the host lacks its content.
    pub fn activate(&self, id: PresetId, force: bool) -> HostResult<Status> {
        let request = self
            .agent
            .post(self.url(&format!("/presets/{id}/activate")));
        self.send_json(request, &Activate { force })
    }

    pub fn installed(&self) -> HostResult<Installed> {
        self.json(self.agent.get(self.url("/content")))
    }

    pub fn plan(&self, manifest: &Manifest) -> HostResult<UploadPlan> {
        self.send_json(self.agent.post(self.url("/content/plan")), manifest)
    }

    /// Sends one file's bytes, read from `body`.
    pub fn upload(&self, path: &ContentPath, sha256: &str, body: impl Read) -> HostResult<()> {
        let mut body = body;
        let response = self
            .agent
            .put(self.url(&format!("/content/files/{path}")))
            .header("Authorization", self.bearer())
            .header(SHA256_HEADER, sha256)
            .header("Content-Type", "application/octet-stream")
            .send(ureq::SendBody::from_reader(&mut body))
            .map_err(|e| self.unreachable(e))?;
        self.read(response).map(|_| ())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn unreachable(&self, error: ureq::Error) -> HostError {
        let host = self.base.strip_suffix(PATH_PREFIX).unwrap_or(&self.base);
        HostError::Unreachable(format!("{host} could not be reached: {error}"))
    }

    fn json<T: DeserializeOwned>(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    ) -> HostResult<T> {
        let response = request
            .header("Authorization", self.bearer())
            .call()
            .map_err(|e| self.unreachable(e))?;
        self.parse(self.read(response)?)
    }

    fn send_json<T: DeserializeOwned>(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithBody>,
        body: &impl Serialize,
    ) -> HostResult<T> {
        let body = serde_json::to_vec(body).expect("protocol types serialise");
        let response = request
            .header("Authorization", self.bearer())
            .header("Content-Type", "application/json")
            .send(&body[..])
            .map_err(|e| self.unreachable(e))?;
        self.parse(self.read(response)?)
    }

    /// The body of a success, or the refusal in it.
    fn read(&self, mut response: ureq::http::Response<ureq::Body>) -> HostResult<String> {
        let status = response.status();
        let text = response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| self.unreachable(e))?;
        if status.is_success() {
            return Ok(text);
        }
        let refusal: Option<ApiError> = serde_json::from_str(&text).ok();
        Err(match refusal {
            Some(ApiError { message, fields }) => HostError::Refused {
                status: status.as_u16(),
                message,
                fields,
            },
            // Something in front of the agent, such as a proxy, answered.
            None => HostError::Refused {
                status: status.as_u16(),
                message: match status.as_u16() {
                    401 => "The host does not know that token.".to_owned(),
                    404 => "No host agent answers at that address.".to_owned(),
                    code => format!("The host answered {code}."),
                },
                fields: Vec::new(),
            },
        })
    }

    fn parse<T: DeserializeOwned>(&self, text: String) -> HostResult<T> {
        // An empty success, such as a delete's, reads as JSON null.
        let text = if text.trim().is_empty() {
            "null"
        } else {
            &text
        };
        serde_json::from_str(text).map_err(|e| {
            HostError::Unreachable(format!("The host answered something unexpected: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read_however_they_are_typed() {
        for (typed, url) in [
            ("host.example", "https://host.example"),
            ("  host.example:3200/ ", "https://host.example:3200"),
            ("http://10.0.0.5:3200", "http://10.0.0.5:3200"),
            (
                "HTTPS://host.example/agent/v1/",
                "https://host.example/agent",
            ),
            ("https://[::1]:3200", "https://[::1]:3200"),
        ] {
            assert_eq!(normalise_url(typed).as_deref(), Some(url), "{typed}");
        }
        for bad in [
            "",
            "   ",
            "ftp://host",
            "https://",
            "host name",
            "user@host",
        ] {
            assert_eq!(normalise_url(bad), None, "{bad}");
        }
        assert_eq!(host_name("https://box.example:3200/agent"), "box.example");
    }

    #[test]
    fn hosts_survive_a_restart_and_keep_their_token_private() {
        let dir = std::env::temp_dir().join(format!("muretto-hosts-{}", std::process::id()));
        let path = dir.join("hosts.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut hosts = Hosts::load(Some(path.clone()));
        assert!(hosts.hosts.is_empty());
        hosts
            .add("", "box.example:3200", "token-one-xxxxxxxx")
            .unwrap();
        hosts
            .add("Club", "http://10.0.0.5:3200", "token-two-xxxxxxxx")
            .unwrap();
        // The same address again takes the new token.
        hosts
            .add("Box", "https://box.example:3200/", " token-new-xxxxxxxx ")
            .unwrap();
        assert!(hosts.add("x", "ftp://nope", "t").is_err());

        let mut hosts = Hosts::load(Some(path.clone()));
        let names: Vec<_> = hosts.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["Box", "Club"]);
        assert_eq!(hosts.hosts[0].token, "token-new-xxxxxxxx");
        assert!(!format!("{:?}", hosts.hosts[0]).contains("token-new"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        hosts.remove("http://10.0.0.5:3200").unwrap();
        assert_eq!(Hosts::load(Some(path)).hosts.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
