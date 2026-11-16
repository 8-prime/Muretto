//! The files a server needs from cars and tracks.
//!
//! The server does not run the game, so it needs only a few small files from
//! each car and track, not the whole folder:
//!
//! - **Required:** each car's `data.acd`. AssettoServer refuses to start
//!   without it.
//! - **Checked:** files the server hashes and compares with each driver's
//!   copy, refusing drivers whose copy differs: the track's `surfaces.ini`
//!   and `models.ini`, its root `.kn5` files, each car's `collider.kn5`, and
//!   the game's own `system/data/surfaces.ini`. If one is missing, that
//!   check is skipped and the server still runs.
//! - **Traffic:** the AI spline, which AssettoServer's traffic drives along.
//!
//! The list comes from AssettoServer's `ChecksumManager`. [`classify`] is
//! the one place that knows it. Muretto runs it over the files in a car or
//! track folder to pick what to upload, and the agent runs it over every
//! upload path to refuse anything else.
//!
//! Paths are relative to the game's root, use `/`, and are case-sensitive on
//! the server. [`classify`] fixes the case of the names the server looks up,
//! such as `data.acd`, so a `Data.ACD` from Windows still lands where the
//! server reads it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::presets::{AiMode, FieldError, Preset, TrackRef};

/// The upload's SHA-256, as lowercase hex. The agent rejects a body that
/// does not match it.
pub const SHA256_HEADER: &str = "x-content-sha256";
/// The largest single file the agent takes. Track `.kn5` files run to
/// hundreds of megabytes.
pub const MAX_FILE: u64 = 4 << 30;

/// A path the server reads, as [`classify`] wrote it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentPath(String);

impl ContentPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ContentPath {
    type Error = String;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        classify(&path)
            .map(|file| file.path)
            .ok_or_else(|| format!("{path} is not a file the server reads"))
    }
}

impl From<ContentPath> for String {
    fn from(path: ContentPath) -> Self {
        path.0
    }
}

/// A file the server reads, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFile {
    pub path: ContentPath,
    pub kind: FileKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    /// `system/data/surfaces.ini`, the game's own surfaces.
    SystemSurfaces,
    /// `content/cars/{car}/data.acd`, or another `data*.acd` when
    /// AssettoServer's alternative checksums are on.
    CarData { car: String },
    /// `content/cars/{car}/collider.kn5`.
    CarCollider { car: String },
    /// `content/tracks/{track}/[{layout}/]data/surfaces.ini`.
    TrackSurfaces {
        track: String,
        layout: Option<String>,
    },
    /// `content/tracks/{track}/models.ini`, or `models_{layout}.ini`.
    TrackModels {
        track: String,
        layout: Option<String>,
    },
    /// A `surfaces.ini` or `.kn5` file in the track's own folder.
    TrackRoot { track: String },
    /// `content/tracks/{track}/[{layout}/]ai/fast_lane.ai` or `.aip`.
    AiSpline {
        track: String,
        layout: Option<String>,
    },
}

/// What happens without a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// The server will not start.
    Required,
    /// The server lets in drivers whose copy differs.
    Checked,
    /// Traffic has nowhere to drive.
    Traffic,
}

impl FileKind {
    pub fn need(&self) -> Need {
        match self {
            FileKind::CarData { .. } => Need::Required,
            FileKind::AiSpline { .. } => Need::Traffic,
            _ => Need::Checked,
        }
    }

    /// The car or track the file belongs to; `None` for the game's own.
    pub fn owner(&self) -> Option<Owner<'_>> {
        match self {
            FileKind::SystemSurfaces => None,
            FileKind::CarData { car } | FileKind::CarCollider { car } => Some(Owner::Car(car)),
            FileKind::TrackSurfaces { track, .. }
            | FileKind::TrackModels { track, .. }
            | FileKind::TrackRoot { track }
            | FileKind::AiSpline { track, .. } => Some(Owner::Track(track)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner<'a> {
    Car(&'a str),
    Track(&'a str),
}

/// Whether the server reads the file at `path`, relative to the game's root,
/// and where it reads it from. Either separator is accepted. `None` for
/// anything else, including any path that would leave its folder.
pub fn classify(path: &str) -> Option<ServerFile> {
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    if !parts.iter().all(|part| folder_name(part)) {
        return None;
    }
    let is = |part: &str, name: &str| part.eq_ignore_ascii_case(name);
    let lower = |part: &str| part.to_ascii_lowercase();
    let owned = |part: &str| part.to_owned();

    let (path, kind) = match parts.as_slice() {
        [s, d, f] if is(s, "system") && is(d, "data") && is(f, "surfaces.ini") => (
            "system/data/surfaces.ini".to_owned(),
            FileKind::SystemSurfaces,
        ),
        [c, cars, car, file] if is(c, "content") && is(cars, "cars") => {
            let file = lower(file);
            let kind = if file.starts_with("data") && file.ends_with(".acd") {
                FileKind::CarData { car: owned(car) }
            } else if file == "collider.kn5" {
                FileKind::CarCollider { car: owned(car) }
            } else {
                return None;
            };
            (format!("content/cars/{car}/{file}"), kind)
        }
        [c, tracks, track, rest @ ..] if is(c, "content") && is(tracks, "tracks") => {
            let track = owned(track);
            let (rest, kind) = match rest {
                [d, f] if is(d, "data") && is(f, "surfaces.ini") => (
                    "data/surfaces.ini".to_owned(),
                    FileKind::TrackSurfaces {
                        track: track.clone(),
                        layout: None,
                    },
                ),
                [layout, d, f] if is(d, "data") && is(f, "surfaces.ini") => (
                    format!("{layout}/data/surfaces.ini"),
                    FileKind::TrackSurfaces {
                        track: track.clone(),
                        layout: Some(owned(layout)),
                    },
                ),
                [a, f] if is(a, "ai") && spline(f) => (
                    format!("ai/{}", lower(f)),
                    FileKind::AiSpline {
                        track: track.clone(),
                        layout: None,
                    },
                ),
                [layout, a, f] if is(a, "ai") && spline(f) => (
                    format!("{layout}/ai/{}", lower(f)),
                    FileKind::AiSpline {
                        track: track.clone(),
                        layout: Some(owned(layout)),
                    },
                ),
                [f] if is(f, "models.ini") => (
                    "models.ini".to_owned(),
                    FileKind::TrackModels {
                        track: track.clone(),
                        layout: None,
                    },
                ),
                // The layout keeps its case: the server builds this name
                // from `CONFIG_TRACK`.
                [f] if let Some(layout) = affixed(f, "models_", ".ini") => (
                    format!("models_{layout}.ini"),
                    FileKind::TrackModels {
                        track: track.clone(),
                        layout: Some(layout.to_owned()),
                    },
                ),
                [f] if is(f, "surfaces.ini") => (
                    "surfaces.ini".to_owned(),
                    FileKind::TrackRoot {
                        track: track.clone(),
                    },
                ),
                [f] if let Some(name) = affixed(f, "", ".kn5") => (
                    format!("{name}.kn5"),
                    FileKind::TrackRoot {
                        track: track.clone(),
                    },
                ),
                _ => return None,
            };
            (format!("content/tracks/{track}/{rest}"), kind)
        }
        _ => return None,
    };
    Some(ServerFile {
        path: ContentPath(path),
        kind,
    })
}

/// A path segment that stays in its folder on any system.
fn folder_name(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// What lies between `prefix` and `suffix`, both matched without regard to
/// case; `None` unless something does.
fn affixed<'a>(file: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let end = file.len().checked_sub(suffix.len())?;
    let (head, tail) = (file.get(..prefix.len())?, file.get(end..)?);
    let middle = file.get(prefix.len()..end).filter(|m| !m.is_empty())?;
    (head.eq_ignore_ascii_case(prefix) && tail.eq_ignore_ascii_case(suffix)).then_some(middle)
}

fn spline(file: &str) -> bool {
    file.eq_ignore_ascii_case("fast_lane.ai") || file.eq_ignore_ascii_case("fast_lane.aip")
}

/// One file Muretto has, from [`classify`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: ContentPath,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// The body of `POST /content/plan`: the server files of the cars and tracks
/// Muretto wants on the host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub files: Vec<ManifestFile>,
}

/// What the agent wants of a [`Manifest`]: the files it lacks or holds a
/// different copy of. Muretto sends each with `PUT /content/files/{path}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadPlan {
    pub upload: Vec<ContentPath>,
}

/// Whether `hash` is how a [`ManifestFile`] or [`SHA256_HEADER`] writes a
/// SHA-256.
pub fn valid_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The reply to `GET /content`: what the host can run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub cars: Vec<InstalledCar>,
    pub tracks: Vec<InstalledTrack>,
    /// Whether the game's own `surfaces.ini` is there to check against.
    pub system_surfaces: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledCar {
    pub id: String,
    /// Whether the server checks drivers' `collider.kn5`.
    pub collider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledTrack {
    pub id: String,
    pub layouts: Vec<InstalledLayout>,
    /// How many of the track's `.kn5` files the server checks.
    pub models_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledLayout {
    /// `None` for tracks with one layout.
    pub id: Option<String>,
    /// Whether it has an AI spline.
    pub traffic: bool,
}

impl Installed {
    /// What the files in a host's content folder add up to. A car counts once
    /// it has a `data.acd`; a layout once it has a `surfaces.ini` or
    /// `models.ini` of its own.
    pub fn from_files<'a>(files: impl IntoIterator<Item = &'a FileKind>) -> Self {
        let mut installed = Installed::default();
        let mut colliders = Vec::new();
        let mut splines = Vec::new();
        for kind in files {
            match kind {
                FileKind::SystemSurfaces => installed.system_surfaces = true,
                FileKind::CarData { car } => {
                    if installed.car(car).is_none() {
                        installed.cars.push(InstalledCar {
                            id: car.clone(),
                            collider: false,
                        });
                    }
                }
                FileKind::CarCollider { car } => colliders.push(car),
                FileKind::TrackSurfaces { track, layout }
                | FileKind::TrackModels { track, layout } => {
                    let track = installed.track_mut(track);
                    if !track.layouts.iter().any(|l| &l.id == layout) {
                        track.layouts.push(InstalledLayout {
                            id: layout.clone(),
                            traffic: false,
                        });
                    }
                }
                FileKind::TrackRoot { track } => installed.track_mut(track).models_checked += 1,
                FileKind::AiSpline { track, layout } => splines.push((track, layout)),
            }
        }
        for car in colliders {
            if let Some(car) = installed.cars.iter_mut().find(|c| &c.id == car) {
                car.collider = true;
            }
        }
        for (track, layout) in splines {
            if let Some(layout) = installed
                .tracks
                .iter_mut()
                .find(|t| &t.id == track)
                .and_then(|t| t.layouts.iter_mut().find(|l| &l.id == layout))
            {
                layout.traffic = true;
            }
        }
        // Root `surfaces.ini` counts towards the checks, not as a layout.
        installed.tracks.retain(|t| !t.layouts.is_empty());
        installed.cars.sort_by(|a, b| a.id.cmp(&b.id));
        installed.tracks.sort_by(|a, b| a.id.cmp(&b.id));
        installed
    }

    pub fn car(&self, id: &str) -> Option<&InstalledCar> {
        self.cars.iter().find(|car| car.id == id)
    }

    pub fn layout(&self, track: &TrackRef) -> Option<&InstalledLayout> {
        self.tracks
            .iter()
            .find(|t| t.id == track.id)?
            .layouts
            .iter()
            .find(|l| l.id == track.layout)
    }

    fn track_mut(&mut self, id: &str) -> &mut InstalledTrack {
        let at = match self.tracks.iter().position(|t| t.id == id) {
            Some(at) => at,
            None => {
                self.tracks.push(InstalledTrack {
                    id: id.to_owned(),
                    layouts: Vec::new(),
                    models_checked: 0,
                });
                self.tracks.len() - 1
            }
        };
        &mut self.tracks[at]
    }

    /// What a preset needs that the host lacks, in the same shape as
    /// [`Preset::validate`].
    pub fn check(&self, preset: &Preset) -> Vec<FieldError> {
        let mut errors = Vec::new();
        let mut add = |field: String, message: String| errors.push(FieldError { field, message });

        match self.layout(&preset.track) {
            None => add(
                "track".into(),
                format!("The host does not have {}.", track_label(&preset.track)),
            ),
            Some(layout)
                if !layout.traffic && preset.entries.iter().any(|e| e.ai != AiMode::None) =>
            {
                add(
                    "track".into(),
                    format!(
                        "The host has no AI spline for {}.",
                        track_label(&preset.track)
                    ),
                )
            }
            Some(_) => {}
        }
        for (i, entry) in preset.entries.iter().enumerate() {
            if self.car(&entry.model).is_none() {
                add(
                    format!("entries[{i}].model"),
                    format!("The host does not have {}.", entry.model),
                );
            }
        }
        errors
    }
}

fn track_label(track: &TrackRef) -> String {
    match &track.layout {
        Some(layout) => format!("{} ({layout})", track.id),
        None => track.id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::Entry;

    fn path(path: &str) -> Option<String> {
        classify(path).map(|file| file.path.0)
    }

    #[test]
    fn the_server_files_of_a_car() {
        let file = classify(r"content\cars\ks_audi_r8_lms\Data.ACD").unwrap();
        assert_eq!(file.path.as_str(), "content/cars/ks_audi_r8_lms/data.acd");
        assert_eq!(file.kind.need(), Need::Required);
        assert_eq!(
            path("content/cars/ks_audi_r8_lms/data_alt.acd").as_deref(),
            Some("content/cars/ks_audi_r8_lms/data_alt.acd")
        );
        assert_eq!(
            classify("content/cars/ks_audi_r8_lms/collider.kn5")
                .unwrap()
                .kind,
            FileKind::CarCollider {
                car: "ks_audi_r8_lms".into()
            }
        );
        assert_eq!(path("content/cars/ks_audi_r8_lms/ks_audi_r8_lms.kn5"), None);
        assert_eq!(
            path("content/cars/ks_audi_r8_lms/skins/red/livery.png"),
            None
        );
    }

    #[test]
    fn the_server_files_of_a_track() {
        let t = "content/tracks/ks_brands_hatch";
        for (from, to) in [
            ("data/surfaces.ini", "data/surfaces.ini"),
            ("gp/data/Surfaces.ini", "gp/data/surfaces.ini"),
            ("Models.ini", "models.ini"),
            ("MODELS_gp.INI", "models_gp.ini"),
            ("surfaces.ini", "surfaces.ini"),
            ("brands_hatch.KN5", "brands_hatch.kn5"),
            ("gp/ai/fast_lane.AIP", "gp/ai/fast_lane.aip"),
            ("ai/fast_lane.ai", "ai/fast_lane.ai"),
        ] {
            assert_eq!(
                path(&format!("{t}/{from}")),
                Some(format!("{t}/{to}")),
                "{from}"
            );
        }
        for skipped in [
            "gp/data/cameras.ini",
            "ui/ui_track.json",
            "gp/models.ini",
            "skins/x.kn5",
        ] {
            assert_eq!(path(&format!("{t}/{skipped}")), None, "{skipped}");
        }
    }

    #[test]
    fn paths_cannot_leave_the_content_folder() {
        for bad in [
            "content/cars/../../etc/data.acd",
            "content/cars/./data.acd",
            "/content/cars/x/data.acd",
            "content//cars/x/data.acd",
            "C:/content/cars/x/data.acd",
            "content/tracks/x/../models.ini",
            "content/cars/x\n/data.acd",
        ] {
            assert_eq!(path(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn manifests_refuse_files_the_server_does_not_read() {
        let json = r#"{"files":[{"path":"content/cars/x/x.kn5","size":1,"sha256":"00"}]}"#;
        assert!(serde_json::from_str::<Manifest>(json).is_err());
    }

    fn installed() -> Installed {
        let kinds: Vec<FileKind> = [
            "content/cars/ks_audi_r8_lms/data.acd",
            "content/cars/ks_audi_r8_lms/collider.kn5",
            "content/cars/half_uploaded/collider.kn5",
            "content/tracks/ks_brands_hatch/gp/data/surfaces.ini",
            "content/tracks/ks_brands_hatch/models_gp.ini",
            "content/tracks/ks_brands_hatch/models_indy.ini",
            "content/tracks/ks_brands_hatch/brands_hatch.kn5",
            "content/tracks/shutoko/data/surfaces.ini",
            "content/tracks/shutoko/ai/fast_lane.aip",
            "content/tracks/only_root/surfaces.ini",
        ]
        .into_iter()
        .map(|p| classify(p).unwrap().kind)
        .collect();
        Installed::from_files(&kinds)
    }

    #[test]
    fn installed_content_from_the_files() {
        let installed = installed();
        assert_eq!(
            installed.cars,
            [InstalledCar {
                id: "ks_audi_r8_lms".into(),
                collider: true
            }]
        );
        let ids: Vec<_> = installed.tracks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["ks_brands_hatch", "shutoko"]);
        let brands = &installed.tracks[0];
        assert_eq!(brands.models_checked, 1);
        let layouts: Vec<_> = brands.layouts.iter().map(|l| l.id.as_deref()).collect();
        assert_eq!(layouts, [Some("gp"), Some("indy")]);
        assert_eq!(
            installed.tracks[1].layouts,
            [InstalledLayout {
                id: None,
                traffic: true
            }]
        );
    }

    #[test]
    fn presets_are_checked_against_what_the_host_has() {
        let entry = |model: &str, ai| Entry {
            model: model.into(),
            skin: None,
            driver_name: None,
            team: None,
            guid: None,
            ballast_kg: 0,
            restrictor: 0,
            ai,
        };
        let installed = installed();
        let track = |id: &str, layout: Option<&str>| TrackRef {
            id: id.into(),
            layout: layout.map(Into::into),
        };

        let ok = Preset::new(
            "Traffic",
            track("shutoko", None),
            vec![entry("ks_audi_r8_lms", AiMode::Auto)],
        );
        assert_eq!(installed.check(&ok), []);

        let missing = Preset::new(
            "Missing",
            track("ks_brands_hatch", Some("gp")),
            vec![
                entry("ks_audi_r8_lms", AiMode::Fixed),
                entry("half_uploaded", AiMode::None),
            ],
        );
        let fields: Vec<_> = installed
            .check(&missing)
            .into_iter()
            .map(|e| e.field)
            .collect();
        assert_eq!(fields, ["track", "entries[1].model"]);

        let no_layout = Preset::new(
            "No layout",
            track("ks_brands_hatch", None),
            vec![entry("ks_audi_r8_lms", AiMode::None)],
        );
        assert_eq!(installed.check(&no_layout).len(), 1);
    }
}
