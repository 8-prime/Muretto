//! What Muretto and its host agent say to each other.
//!
//! The host agent runs next to AssettoServer, in the same container. It keeps
//! a library of [presets](presets), writes the active one into the server's
//! `cfg/` folder and restarts the server. It also keeps the few
//! [content](content) files the server reads from each car and track, which
//! Muretto uploads from the player's own install.
//!
//! Everything is JSON over HTTP under [`PATH_PREFIX`], authorised with a
//! bearer token the host's owner sets:
//!
//! | Method   | Path                          | Body → Reply                                      |
//! |----------|-------------------------------|---------------------------------------------------|
//! | `GET`    | `/status`                     | → [`Status`]                                      |
//! | `GET`    | `/presets`                    | → `Vec<PresetSummary>`                            |
//! | `GET`    | `/presets/{id}`               | → [`StoredPreset`](presets::StoredPreset)         |
//! | `PUT`    | `/presets/{id}`               | [`Preset`](presets::Preset) → `StoredPreset`      |
//! | `DELETE` | `/presets/{id}`               |                                                   |
//! | `POST`   | `/presets/{id}/activate`      | [`Activate`](presets::Activate) →                 |
//! | `GET`    | `/content`                    | → [`Installed`](content::Installed)               |
//! | `POST`   | `/content/plan`               | [`Manifest`](content::Manifest) → [`UploadPlan`](content::UploadPlan) |
//! | `PUT`    | `/content/files/{path}`       | the file's bytes, [`SHA256_HEADER`](content::SHA256_HEADER) |
//!
//! `PUT /presets/{id}` takes `If-Match: <revision>` to replace a preset and
//! `If-None-Match: *` to create one; a stale revision gets `409 Conflict`.
//! Muretto picks the id with [`PresetId::new`](presets::PresetId::new), so
//! repeating a create after a timeout cannot make a second copy.
//!
//! Activating a preset that does not fit the host's content gets `422` with
//! the fields at fault; activating while players are on the server, without
//! [`force`](presets::Activate::force), gets `409`.
//!
//! Requests the agent refuses get a 4xx status and an [`ApiError`].

use serde::{Deserialize, Serialize};

use crate::presets::PresetId;

pub mod content;
pub mod presets;

/// Every route lives under this.
pub const PATH_PREFIX: &str = "/v1";

/// Why the agent refused a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    /// A sentence to show the player.
    pub message: String,
    /// What was wrong with each field, for a refused preset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<presets::FieldError>,
}

/// The reply to `GET /status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// The preset the server was last started with.
    pub active: Option<ActivePreset>,
    pub server: ServerState,
    /// Drivers on the server, when it answers.
    pub players: Option<u32>,
    pub agent_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivePreset {
    pub id: PresetId,
    pub name: String,
    /// The revision the server runs.
    pub revision: u64,
    /// Whether the preset was saved since: activating it again applies that.
    /// False once the preset is deleted; the server runs on regardless.
    pub changed_since: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ServerState {
    /// No preset is active, so the server does not run.
    Idle,
    /// Seconds since the Unix epoch.
    Running { since: u64 },
    /// It exited on its own; the agent starts it again shortly.
    Crashed {
        exit_code: Option<i32>,
        /// Starts in a row that ended this way.
        restarts: u32,
    },
}
