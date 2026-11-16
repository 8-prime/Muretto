//! Server presets: everything AssettoServer reads from its `cfg/` folder,
//! minus what belongs to the machine.
//!
//! The host keeps its ports, admin password, lobby registration and server
//! name to itself and merges them in when it writes `server_cfg.ini`, so
//! activating someone else's preset cannot move the server or hide it.
//!
//! Values are stored the way a player thinks of them: a time of day rather
//! than a sun angle, an enum rather than AC's numbers, and one entry per car
//! from which the `CARS=` list follows. The agent turns them into files.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The server numbers cars with one byte.
pub const MAX_ENTRIES: usize = 255;
/// The longest preset name, in characters.
pub const MAX_NAME: usize = 64;
/// The longest free text: the welcome message, and the raw YAML and INI a
/// preset passes through.
pub const MAX_TEXT: usize = 64 * 1024;
/// The last second of a day.
const DAY_END: u32 = 24 * 3600 - 1;

/// A preset's id. Muretto makes it, so a retried create replaces the same
/// preset. v7, so ids sort by when they were made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PresetId(Uuid);

impl PresetId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for PresetId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// What Muretto sends to create or replace a preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    pub track: TrackRef,
    /// One per slot, in pit and grid order.
    pub entries: Vec<Entry>,
    pub sessions: Sessions,
    pub rules: Rules,
    pub conditions: Conditions,
    pub access: Access,
    /// AssettoServer's own options, passed through unchanged: its keys change
    /// between releases and its plugins add their own.
    pub extra_cfg_yml: Option<String>,
    /// `csp_extra_options.ini`, if the preset sets any.
    pub csp_extra_options: Option<String>,
}

/// A preset as the agent keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPreset {
    pub id: PresetId,
    /// Goes up by one with every save. Send it back in `If-Match`.
    pub revision: u64,
    /// The name of whoever saved it last, as their token says.
    pub updated_by: String,
    /// Seconds since the Unix epoch.
    pub updated_at: u64,
    pub preset: Preset,
}

/// A line in the preset list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetSummary {
    pub id: PresetId,
    pub revision: u64,
    pub name: String,
    pub track: TrackRef,
    /// Distinct car models, in entry order.
    pub cars: Vec<String>,
    pub slots: usize,
    pub updated_by: String,
    pub updated_at: u64,
    /// Whether the server runs this preset now.
    pub active: bool,
}

/// The body of `POST /presets/{id}/activate`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activate {
    /// Restart even with players on the server. Without it the agent answers
    /// `409 Conflict` while anyone is connected.
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackRef {
    /// The track's folder name: `TRACK`.
    pub id: String,
    /// The layout's folder name: `CONFIG_TRACK`. `None` for tracks with one
    /// layout.
    pub layout: Option<String>,
}

/// One slot: `[CAR_n]` in `entry_list.ini`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The car's folder name.
    pub model: String,
    /// The skin's folder name. `None` lets the driver pick.
    pub skin: Option<String>,
    pub driver_name: Option<String>,
    pub team: Option<String>,
    /// A Steam id that owns this slot; anyone may take it when `None`.
    pub guid: Option<String>,
    pub ballast_kg: u16,
    /// 0–100.
    pub restrictor: u8,
    pub ai: AiMode,
}

/// Whether AssettoServer's traffic may drive a slot. Needs `EnableAi` in
/// `extra_cfg_yml` and a spline on the track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiMode {
    /// Players only.
    #[default]
    None,
    /// Traffic while no player holds the slot.
    Auto,
    /// Traffic only.
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sessions {
    pub booking: Option<Booking>,
    pub practice: Option<Session>,
    pub qualify: Option<Session>,
    pub race: Option<Race>,
    /// Start over from the first session after the last one.
    pub loop_mode: bool,
    /// How long results stay on screen between sessions.
    pub result_screen_secs: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Booking {
    pub minutes: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub minutes: u16,
    /// Whether drivers may join while it runs.
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Race {
    pub length: RaceLength,
    pub join: RaceJoin,
    /// Seconds on the grid before the start.
    pub wait_secs: u16,
    /// Seconds the race goes on after the winner finishes.
    pub over_secs: u16,
    pub pit_window: Option<PitWindow>,
    /// How many of the top finishers of the race before start reversed.
    /// 0 for none.
    pub reversed_grid: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case", deny_unknown_fields)]
pub enum RaceLength {
    Laps { laps: u16 },
    Minutes { minutes: u16, extra_lap: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaceJoin {
    Closed,
    Open,
    /// Open until the race starts.
    BeforeStart,
}

/// The laps a mandatory pit stop has to fall between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PitWindow {
    pub opens_lap: u16,
    pub closes_lap: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub abs: AidRule,
    pub traction_control: AidRule,
    pub stability_control: bool,
    pub autoclutch: bool,
    pub tyre_blankets: bool,
    pub force_virtual_mirror: bool,
    /// Percent of real consumption.
    pub fuel_rate: u16,
    /// 0–100 percent.
    pub damage: u8,
    /// Percent of real wear.
    pub tyre_wear: u16,
    /// How many tyres may leave the track before a cut counts.
    pub tyres_out_allowed: u8,
    /// Compound short names, such as `SM`; empty allows all.
    pub legal_tyres: Vec<String>,
    /// Kicks a driver who touches others this often. `None` for no limit.
    pub max_contacts_per_km: Option<u8>,
    /// Percent of drivers needed to kick someone.
    pub kick_quorum: u8,
    /// Percent of drivers needed for other votes.
    pub vote_quorum: u8,
    pub vote_secs: u16,
    pub start_rule: StartRule,
}

/// Whether a driver aid may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AidRule {
    Denied,
    /// Only on cars that have it in reality.
    Factory,
    /// On every car.
    Forced,
}

/// What happens to a jumped start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartRule {
    /// The car cannot move until the lights go out.
    CarLocked,
    TeleportToPits,
    DriveThrough,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    /// Seconds since midnight, track time.
    pub time_of_day: u32,
    /// How much faster than real time the day goes; 1 for real time.
    pub time_multiplier: u8,
    /// At least one. The server picks one per session.
    pub weather: Vec<Weather>,
    pub grip: TrackGrip,
}

/// `[WEATHER_n]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weather {
    /// The weather's folder name, such as `3_clear`.
    pub graphics: String,
    pub ambient_c: i8,
    /// Added to the ambient temperature to give the road's.
    pub road_offset_c: i8,
    pub ambient_variation_c: u8,
    pub road_variation_c: u8,
    pub wind_min_kmh: u16,
    pub wind_max_kmh: u16,
    /// Where the wind blows from, 0–359.
    pub wind_direction_deg: u16,
    pub wind_direction_variation_deg: u16,
}

/// `[DYNAMIC_TRACK]`: rubber on the racing line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackGrip {
    /// Grip at the start of the first session, 0–100 percent.
    pub start_percent: u8,
    pub randomness: u8,
    /// Laps driven, across all cars, for each percent gained.
    pub lap_gain: u16,
    /// Percent of the gained grip that carries into the next session.
    pub transfer_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    /// Asked of everyone joining. `None` for an open server.
    pub password: Option<String>,
    /// At most the number of entries; `None` for one client per entry.
    pub max_clients: Option<u8>,
    /// Drivers take any free slot of the car they pick, instead of booking.
    pub pickup_mode: bool,
    /// Only the Steam ids in the entries may join.
    pub locked_entry_list: bool,
    /// Shown on joining; may span lines.
    pub welcome_message: Option<String>,
}

/// One thing wrong with a preset, for Muretto to show next to the field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    /// Where, as a path like `entries[3].model`.
    pub field: String,
    pub message: String,
}

impl Preset {
    /// A preset with nothing but practice and the usual rules: what Muretto's
    /// editor starts from.
    pub fn new(name: impl Into<String>, track: TrackRef, entries: Vec<Entry>) -> Self {
        Self {
            name: name.into(),
            track,
            entries,
            sessions: Sessions {
                booking: None,
                practice: Some(Session {
                    minutes: 30,
                    open: true,
                }),
                qualify: None,
                race: None,
                loop_mode: true,
                result_screen_secs: 60,
            },
            rules: Rules {
                abs: AidRule::Factory,
                traction_control: AidRule::Factory,
                stability_control: false,
                autoclutch: true,
                tyre_blankets: true,
                force_virtual_mirror: false,
                fuel_rate: 100,
                damage: 50,
                tyre_wear: 100,
                tyres_out_allowed: 2,
                legal_tyres: Vec::new(),
                max_contacts_per_km: None,
                kick_quorum: 85,
                vote_quorum: 80,
                vote_secs: 20,
                start_rule: StartRule::DriveThrough,
            },
            conditions: Conditions {
                time_of_day: 14 * 3600,
                time_multiplier: 1,
                weather: vec![Weather {
                    graphics: "3_clear".into(),
                    ambient_c: 22,
                    road_offset_c: 6,
                    ambient_variation_c: 1,
                    road_variation_c: 1,
                    wind_min_kmh: 0,
                    wind_max_kmh: 5,
                    wind_direction_deg: 0,
                    wind_direction_variation_deg: 30,
                }],
                grip: TrackGrip {
                    start_percent: 98,
                    randomness: 2,
                    lap_gain: 700,
                    transfer_percent: 80,
                },
            },
            access: Access {
                password: None,
                max_clients: None,
                pickup_mode: true,
                locked_entry_list: false,
                welcome_message: None,
            },
            extra_cfg_yml: None,
            csp_extra_options: None,
        }
    }

    /// Distinct car models, in entry order: the server's `CARS=`.
    pub fn cars(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.entries
            .iter()
            .map(|entry| entry.model.as_str())
            .filter(|model| seen.insert(*model))
            .collect()
    }

    /// What is wrong with the preset on its own terms. Whether its cars,
    /// track and weather are installed is for the host to check against what
    /// it has.
    ///
    /// Every text that ends up in an INI file is checked for what would break
    /// out of its line or list, since the files are written from it.
    pub fn validate(&self) -> Vec<FieldError> {
        let mut errors = Errors::default();

        if self.name.trim().is_empty() {
            errors.add("name", "Give the preset a name.");
        } else if self.name.chars().count() > MAX_NAME {
            errors.add("name", format!("Keep the name to {MAX_NAME} characters."));
        }
        errors.text("name", &self.name);

        errors.folder("track.id", &self.track.id);
        if let Some(layout) = &self.track.layout {
            errors.folder("track.layout", layout);
        }

        if self.entries.is_empty() {
            errors.add("entries", "Add at least one car.");
        } else if self.entries.len() > MAX_ENTRIES {
            errors.add(
                "entries",
                format!("A server holds at most {MAX_ENTRIES} cars."),
            );
        }
        for (i, entry) in self.entries.iter().enumerate() {
            let at = |field: &str| format!("entries[{i}].{field}");
            errors.folder(&at("model"), &entry.model);
            if let Some(skin) = &entry.skin {
                errors.folder(&at("skin"), skin);
            }
            for (field, value) in [("driver_name", &entry.driver_name), ("team", &entry.team)] {
                if let Some(value) = value {
                    errors.text(&at(field), value);
                }
            }
            if let Some(guid) = &entry.guid
                && !(guid.len() == 17 && guid.bytes().all(|b| b.is_ascii_digit()))
            {
                errors.add(&at("guid"), "A Steam id is 17 digits.");
            }
            if entry.restrictor > 100 {
                errors.add(&at("restrictor"), "The restrictor goes up to 100.");
            }
        }

        let sessions = &self.sessions;
        if sessions.practice.is_none() && sessions.qualify.is_none() && sessions.race.is_none() {
            errors.add("sessions", "Add a practice, qualifying or race session.");
        }
        for (field, session) in [
            ("sessions.practice", sessions.practice),
            ("sessions.qualify", sessions.qualify),
        ] {
            if session.is_some_and(|session| session.minutes == 0) {
                errors.add(
                    &format!("{field}.minutes"),
                    "A session needs at least a minute.",
                );
            }
        }
        if sessions.booking.is_some_and(|booking| booking.minutes == 0) {
            errors.add(
                "sessions.booking.minutes",
                "Booking needs at least a minute.",
            );
        }
        if let Some(race) = &sessions.race {
            match race.length {
                RaceLength::Laps { laps: 0 } => {
                    errors.add("sessions.race.length", "A race needs at least a lap.")
                }
                RaceLength::Minutes { minutes: 0, .. } => {
                    errors.add("sessions.race.length", "A race needs at least a minute.")
                }
                _ => {}
            }
            if let Some(window) = race.pit_window
                && window.opens_lap >= window.closes_lap
            {
                errors.add(
                    "sessions.race.pit_window",
                    "The pit window has to close after it opens.",
                );
            }
            if usize::from(race.reversed_grid) > self.entries.len() {
                errors.add(
                    "sessions.race.reversed_grid",
                    "Reverse no more places than there are cars.",
                );
            }
        }

        let rules = &self.rules;
        if rules.damage > 100 {
            errors.add("rules.damage", "Damage goes up to 100%.");
        }
        if rules.tyres_out_allowed > 4 {
            errors.add("rules.tyres_out_allowed", "A car has four tyres.");
        }
        for (field, quorum) in [
            ("rules.kick_quorum", rules.kick_quorum),
            ("rules.vote_quorum", rules.vote_quorum),
        ] {
            if quorum > 100 {
                errors.add(field, "A quorum goes up to 100%.");
            }
        }
        for (i, tyre) in rules.legal_tyres.iter().enumerate() {
            let field = format!("rules.legal_tyres[{i}]");
            if tyre.is_empty() || tyre.contains(';') {
                errors.add(&field, "That is not a tyre compound.");
            }
            errors.text(&field, tyre);
        }

        let conditions = &self.conditions;
        if conditions.time_of_day > DAY_END {
            errors.add("conditions.time_of_day", "That is not a time of day.");
        }
        if conditions.time_multiplier == 0 {
            errors.add("conditions.time_multiplier", "Time has to move.");
        }
        if conditions.weather.is_empty() {
            errors.add("conditions.weather", "Add a weather.");
        }
        for (i, weather) in conditions.weather.iter().enumerate() {
            let at = |field: &str| format!("conditions.weather[{i}].{field}");
            errors.folder(&at("graphics"), &weather.graphics);
            if weather.wind_min_kmh > weather.wind_max_kmh {
                errors.add(
                    &at("wind_max_kmh"),
                    "The strongest wind is below the weakest.",
                );
            }
            if weather.wind_direction_deg >= 360 {
                errors.add(&at("wind_direction_deg"), "Directions go from 0 to 359.");
            }
            if weather.wind_direction_variation_deg > 180 {
                errors.add(
                    &at("wind_direction_variation_deg"),
                    "The wind can turn at most 180° either way.",
                );
            }
        }
        let grip = conditions.grip;
        if grip.start_percent > 100 {
            errors.add("conditions.grip.start_percent", "Grip goes up to 100%.");
        }
        if grip.transfer_percent > 100 {
            errors.add(
                "conditions.grip.transfer_percent",
                "Grip carried over goes up to 100%.",
            );
        }

        let access = &self.access;
        if let Some(password) = &access.password {
            if password.is_empty() {
                errors.add(
                    "access.password",
                    "Leave the password out instead of empty.",
                );
            }
            errors.text("access.password", password);
        }
        if let Some(max) = access.max_clients
            && (max == 0 || usize::from(max) > self.entries.len())
        {
            errors.add(
                "access.max_clients",
                "Allow between one client and one per car.",
            );
        }
        if let Some(message) = &access.welcome_message {
            errors.long_text("access.welcome_message", message);
        }

        if let Some(yaml) = &self.extra_cfg_yml {
            errors.long_text("extra_cfg_yml", yaml);
        }
        if let Some(ini) = &self.csp_extra_options {
            errors.long_text("csp_extra_options", ini);
        }

        errors.0
    }
}

impl StoredPreset {
    pub fn summary(&self, active: bool) -> PresetSummary {
        PresetSummary {
            id: self.id,
            revision: self.revision,
            name: self.preset.name.clone(),
            track: self.preset.track.clone(),
            cars: self.preset.cars().into_iter().map(str::to_owned).collect(),
            slots: self.preset.entries.len(),
            updated_by: self.updated_by.clone(),
            updated_at: self.updated_at,
            active,
        }
    }
}

#[derive(Default)]
struct Errors(Vec<FieldError>);

impl Errors {
    fn add(&mut self, field: &str, message: impl Into<String>) {
        self.0.push(FieldError {
            field: field.to_owned(),
            message: message.into(),
        });
    }

    /// One line of an INI value.
    fn text(&mut self, field: &str, value: &str) {
        if value.chars().any(char::is_control) {
            self.add(field, "Line breaks and control characters cannot go here.");
        }
    }

    /// Text written to a file of its own, so it may span lines.
    fn long_text(&mut self, field: &str, value: &str) {
        if value.len() > MAX_TEXT {
            self.add(field, format!("Keep this under {} KiB.", MAX_TEXT / 1024));
        }
        if value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            self.add(field, "Control characters cannot go here.");
        }
    }

    /// A folder name the server joins into paths and lists: `CARS=a;b`,
    /// `content/cars/{model}`.
    fn folder(&mut self, field: &str, value: &str) {
        let bad = value.is_empty()
            || value == "."
            || value == ".."
            || value
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ';' | ',' | '=' | '[' | ']'));
        if bad {
            self.add(field, "That is not a content folder name.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &str) -> Entry {
        Entry {
            model: model.into(),
            skin: None,
            driver_name: None,
            team: None,
            guid: None,
            ballast_kg: 0,
            restrictor: 0,
            ai: AiMode::None,
        }
    }

    fn preset() -> Preset {
        Preset::new(
            "Sunday GT3",
            TrackRef {
                id: "ks_brands_hatch".into(),
                layout: Some("gp".into()),
            },
            vec![
                entry("ks_porsche_911_gt3_r_2016"),
                entry("ks_audi_r8_lms"),
                entry("ks_porsche_911_gt3_r_2016"),
            ],
        )
    }

    fn fields(preset: &Preset) -> Vec<String> {
        preset.validate().into_iter().map(|e| e.field).collect()
    }

    #[test]
    fn the_editor_default_is_valid() {
        assert_eq!(preset().validate(), []);
    }

    #[test]
    fn cars_are_the_distinct_models_in_entry_order() {
        assert_eq!(
            preset().cars(),
            ["ks_porsche_911_gt3_r_2016", "ks_audi_r8_lms"]
        );
    }

    #[test]
    fn presets_survive_json() {
        let mut preset = preset();
        preset.sessions.race = Some(Race {
            length: RaceLength::Minutes {
                minutes: 20,
                extra_lap: true,
            },
            join: RaceJoin::BeforeStart,
            wait_secs: 60,
            over_secs: 120,
            pit_window: Some(PitWindow {
                opens_lap: 3,
                closes_lap: 8,
            }),
            reversed_grid: 0,
        });
        preset.extra_cfg_yml = Some("EnableAi: false\n".into());
        let json = serde_json::to_string(&preset).unwrap();
        assert_eq!(serde_json::from_str::<Preset>(&json).unwrap(), preset);
        assert!(json.contains(r#""length":{"unit":"minutes","minutes":20,"extra_lap":true}"#));
    }

    #[test]
    fn unknown_fields_are_refused_rather_than_dropped() {
        // A newer Muretto talking to an older agent must not lose settings
        // silently on save.
        let mut json = serde_json::to_value(preset()).unwrap();
        json["rules"]["new_rule"] = true.into();
        assert!(serde_json::from_value::<Preset>(json).is_err());
    }

    #[test]
    fn text_cannot_break_out_of_its_ini_line() {
        let mut preset = preset();
        preset.name = "Fun\nADMIN_PASSWORD=x".into();
        preset.access.password = Some("a\rb".into());
        preset.entries[0].driver_name = Some("x\n[CAR_9]".into());
        assert_eq!(
            fields(&preset),
            ["name", "entries[0].driver_name", "access.password"]
        );
    }

    #[test]
    fn folder_names_cannot_leave_their_folder_or_list() {
        let mut preset = preset();
        preset.track.id = "../../etc".into();
        preset.entries[1].model = "a;b".into();
        preset.entries[2].skin = Some("..".into());
        preset.conditions.weather[0].graphics = String::new();
        assert_eq!(
            fields(&preset),
            [
                "track.id",
                "entries[1].model",
                "entries[2].skin",
                "conditions.weather[0].graphics"
            ]
        );
    }

    #[test]
    fn clients_and_sessions_fit_the_entries() {
        let mut preset = preset();
        preset.access.max_clients = Some(4);
        preset.sessions.practice = None;
        assert_eq!(fields(&preset), ["sessions", "access.max_clients"]);
    }

    #[test]
    fn ids_read_back_what_they_print() {
        let id = PresetId::new();
        assert_eq!(id.to_string().parse::<PresetId>().unwrap(), id);
        assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{id}\""));
    }
}
