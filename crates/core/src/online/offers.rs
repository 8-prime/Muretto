//! Where a server says its cars and track can be downloaded.
//!
//! AssettoServer and Content Manager's server wrapper publish an extended
//! description on the port named after `ℹ` in the server's name. Its
//! `content` lists, per car and for the track, a link and a version. Links
//! go anywhere: straight to an archive, or to a page on Google Drive, Mega
//! or MediaFire that has to be visited in a browser.

use std::time::Duration;

use serde_json::Value;

use super::{LOBBY_AGENT, Server, get};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Car,
    Track,
}

/// One download a server offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub kind: Kind,
    /// The car's folder name; empty for the track.
    pub id: String,
    pub version: Option<String>,
    pub url: String,
}

impl Offer {
    /// Whether the link is the archive itself, which Muretto can download,
    /// rather than a page about it.
    pub fn is_direct(&self) -> bool {
        archive_name(&self.url).is_some()
    }

    /// Where the link points, for showing: `mega.nz`.
    pub fn host(&self) -> &str {
        let rest = self
            .url
            .split_once("://")
            .map_or(&*self.url, |(_, rest)| rest);
        rest.split(['/', '?', '#']).next().unwrap_or(rest)
    }
}

/// The file name a link ends in, if it names an archive.
pub fn archive_name(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    let lower = name.to_ascii_lowercase();
    [".zip", ".7z", ".rar"]
        .iter()
        .any(|extension| lower.ends_with(extension) && lower.len() > extension.len())
        .then(|| percent_decode(name))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads the `content` of a server's extended description. Entries without
/// a usable link are left out.
pub fn parse_details(text: &str) -> Result<Vec<Offer>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::Network(format!("The server sent something unexpected: {e}")))?;
    let Some(content) = value.get("content").filter(|c| c.is_object()) else {
        return Ok(Vec::new());
    };
    let offer = |kind, id: &str, entry: &Value| {
        let url = entry.get("url")?.as_str()?.trim();
        // "" and "#" stand for "no link" in the wild.
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return None;
        }
        Some(Offer {
            kind,
            id: id.to_owned(),
            version: entry
                .get("version")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
            url: url.to_owned(),
        })
    };
    let mut offers: Vec<Offer> = content
        .get("cars")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(id, entry)| offer(Kind::Car, id, entry))
        .collect();
    if let Some(track) = content.get("track") {
        offers.extend(offer(Kind::Track, "", track));
    }
    Ok(offers)
}

/// Asks the server where its content can be downloaded. A server without
/// an extended description offers nothing.
pub fn fetch_offers(server: &Server) -> Result<Vec<Offer>> {
    let Some(port) = server.details_port else {
        return Ok(Vec::new());
    };
    let url = format!("http://{}:{port}/api/details", server.ip);
    parse_details(&get(&url, LOBBY_AGENT, Duration::from_secs(6))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_read_and_the_unusable_ones_left_out() {
        let offers = parse_details(
            r##"{"name":"x","content":{
                "cars":{
                  "vdc_bmw_e36":{"version":"5.0","url":"http://cdn.example.net/cars/vdc_bmw_e36.7z"},
                  "skyline":{"url":"https://mega.nz/file/abc#key"},
                  "nothing":{"url":""},
                  "hash":{"version":"1","url":"#"}},
                "track":{"version":"v0.9.1","url":"http://example.net/tracks/Shutoko%20Revival.7z"}}}"##,
        )
        .unwrap();
        assert_eq!(offers.len(), 3);
        let car = offers.iter().find(|o| o.id == "vdc_bmw_e36").unwrap();
        assert!(car.is_direct());
        assert_eq!(car.version.as_deref(), Some("5.0"));
        let page = offers.iter().find(|o| o.id == "skyline").unwrap();
        assert!(!page.is_direct());
        assert_eq!(page.host(), "mega.nz");
        let track = offers.iter().find(|o| o.kind == Kind::Track).unwrap();
        assert_eq!(
            archive_name(&track.url).as_deref(),
            Some("Shutoko Revival.7z")
        );
        assert!(parse_details(r#"{"content":null}"#).unwrap().is_empty());
    }
}
