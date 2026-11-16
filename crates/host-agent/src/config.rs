//! The agent's settings, from the environment.

use std::{env, net::SocketAddr, path::PathBuf};

/// Everything the agent needs to know about its machine.
#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    /// AssettoServer's working directory: `cfg/`, `content/` and `system/`.
    pub server_dir: PathBuf,
    /// Where the agent keeps its presets.
    pub state_dir: PathBuf,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub host: Host,
    pub tokens: Tokens,
}

/// What belongs to the machine rather than a preset. Merged into every
/// `server_cfg.ini` the agent writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    /// Comes before the preset's name in the lobby. Empty for the preset's
    /// name alone.
    pub name: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub http_port: u16,
    pub admin_password: Option<String>,
    pub register_to_lobby: bool,
    /// The most clients the machine takes, whatever a preset asks.
    pub max_clients: Option<u8>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            name: String::new(),
            tcp_port: 9600,
            udp_port: 9600,
            http_port: 8081,
            admin_password: None,
            register_to_lobby: true,
            max_clients: None,
        }
    }
}

/// Who may use the agent: a name for each bearer token.
#[derive(Clone, Default)]
pub struct Tokens(Vec<(String, String)>);

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.0.iter().map(|(name, _)| name).collect();
        f.debug_tuple("Tokens").field(&names).finish()
    }
}

/// Tokens shorter than this are refused: they guard a process restart and
/// file uploads.
pub const MIN_TOKEN: usize = 16;

impl Tokens {
    /// Reads `name:token,name:token`.
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut tokens = Vec::new();
        for pair in list
            .split(',')
            .map(str::trim)
            .filter(|pair| !pair.is_empty())
        {
            let (name, token) = pair
                .split_once(':')
                .ok_or_else(|| format!("`{pair}` is not `name:token`"))?;
            let (name, token) = (name.trim(), token.trim());
            if name.is_empty() || name.chars().any(char::is_control) {
                return Err(format!("`{name}` is not a name"));
            }
            if token.len() < MIN_TOKEN {
                return Err(format!(
                    "{name}'s token is shorter than {MIN_TOKEN} characters"
                ));
            }
            tokens.push((name.to_owned(), token.to_owned()));
        }
        if tokens.is_empty() {
            return Err("no tokens".into());
        }
        Ok(Self(tokens))
    }

    /// The name that goes with `token`.
    pub fn name(&self, token: &str) -> Option<&str> {
        // Every token is compared in full, so timing tells nothing about
        // which one nearly matched.
        let mut found = None;
        for (name, known) in &self.0 {
            if same(known.as_bytes(), token.as_bytes()) {
                found = Some(name.as_str());
            }
        }
        found
    }
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let var = |key: &str| env::var(key).ok().filter(|value| !value.is_empty());
        fn parsed<T: std::str::FromStr>(
            key: &str,
            value: Option<String>,
            default: T,
        ) -> Result<T, String> {
            match value {
                Some(value) => value
                    .parse()
                    .map_err(|_| format!("{key}: `{value}` is not valid")),
                None => Ok(default),
            }
        }

        let tokens = Tokens::parse(&var("AGENT_TOKENS").ok_or("AGENT_TOKENS is not set")?)
            .map_err(|error| format!("AGENT_TOKENS: {error}"))?;
        let server_dir = PathBuf::from(var("SERVER_DIR").unwrap_or_else(|| ".".into()));
        let state_dir = var("AGENT_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| server_dir.join("muretto"));
        let defaults = Host::default();
        let host = Host {
            name: var("SERVER_NAME").unwrap_or_default(),
            tcp_port: parsed("SERVER_TCP_PORT", var("SERVER_TCP_PORT"), defaults.tcp_port)?,
            udp_port: parsed("SERVER_UDP_PORT", var("SERVER_UDP_PORT"), defaults.udp_port)?,
            http_port: parsed(
                "SERVER_HTTP_PORT",
                var("SERVER_HTTP_PORT"),
                defaults.http_port,
            )?,
            admin_password: var("SERVER_ADMIN_PASSWORD"),
            register_to_lobby: parsed::<u8>(
                "SERVER_REGISTER_TO_LOBBY",
                var("SERVER_REGISTER_TO_LOBBY"),
                1,
            )? != 0,
            max_clients: var("SERVER_MAX_CLIENTS")
                .map(|value| parsed("SERVER_MAX_CLIENTS", Some(value), 0))
                .transpose()?,
        };
        if let Some(text) = [&Some(host.name.clone()), &host.admin_password]
            .into_iter()
            .flatten()
            .find(|text| text.chars().any(char::is_control))
        {
            return Err(format!("`{text}` cannot hold line breaks"));
        }

        Ok(Self {
            addr: parsed(
                "AGENT_ADDR",
                var("AGENT_ADDR"),
                SocketAddr::from(([127, 0, 0, 1], 3200)),
            )?,
            server_dir,
            state_dir,
            command: PathBuf::from(
                var("SERVER_COMMAND").unwrap_or_else(|| "/app/AssettoServer".into()),
            ),
            args: var("SERVER_ARGS")
                .unwrap_or_else(|| "--plugins-from-workdir".into())
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            host,
            tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_name_their_owner() {
        let tokens = Tokens::parse("stefan:0123456789abcdef, anna : fedcba9876543210").unwrap();
        assert_eq!(tokens.name("fedcba9876543210"), Some("anna"));
        assert_eq!(tokens.name("0123456789abcdeX"), None);
        assert_eq!(tokens.name(""), None);
    }

    #[test]
    fn short_or_missing_tokens_are_refused() {
        assert!(Tokens::parse("stefan:short").is_err());
        assert!(Tokens::parse("0123456789abcdef").is_err());
        assert!(Tokens::parse(" , ").is_err());
    }

    #[test]
    fn token_lists_do_not_print_their_tokens() {
        let tokens = Tokens::parse("stefan:0123456789abcdef").unwrap();
        assert!(!format!("{tokens:?}").contains("0123"));
    }
}
