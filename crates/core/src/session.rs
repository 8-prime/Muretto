//! A single-player session as the player configures it, and its translation
//! into the `race.ini` and `assists.ini` the game reads on start.

use crate::{
    content::{Car, Library, road_temperature},
    ini::Ini,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionMode {
    Practice,
    Hotlap,
    TimeAttack,
    Race,
    Weekend,
    TrackDay,
    Drift,
    Drag,
}

impl SessionMode {
    pub const ALL: [SessionMode; 8] = [
        SessionMode::Practice,
        SessionMode::Hotlap,
        SessionMode::TimeAttack,
        SessionMode::Race,
        SessionMode::Weekend,
        SessionMode::TrackDay,
        SessionMode::Drift,
        SessionMode::Drag,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SessionMode::Practice => "Practice",
            SessionMode::Hotlap => "Hotlap",
            SessionMode::TimeAttack => "Time attack",
            SessionMode::Race => "Race",
            SessionMode::Weekend => "Weekend",
            SessionMode::TrackDay => "Track day",
            SessionMode::Drift => "Drift",
            SessionMode::Drag => "Drag",
        }
    }

    /// Whether other cars are on track.
    pub fn has_opponents(self) -> bool {
        matches!(
            self,
            SessionMode::Race | SessionMode::Weekend | SessionMode::TrackDay | SessionMode::Drag
        )
    }

    /// Whether the session ends after a set number of laps.
    pub fn has_laps(self) -> bool {
        matches!(
            self,
            SessionMode::Race | SessionMode::Weekend | SessionMode::Drag
        )
    }

    /// Whether the player picks a grid slot.
    pub fn has_grid(self) -> bool {
        matches!(self, SessionMode::Race)
    }
}

/// Which cars fill the rest of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridKind {
    SameCar,
    SameClass,
}

/// How much rubber is on the racing line and how fast it builds up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSurface {
    Dusty,
    Old,
    Slow,
    Green,
    Fast,
    Optimum,
}

impl TrackSurface {
    pub const ALL: [TrackSurface; 6] = [
        TrackSurface::Dusty,
        TrackSurface::Old,
        TrackSurface::Slow,
        TrackSurface::Green,
        TrackSurface::Fast,
        TrackSurface::Optimum,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrackSurface::Dusty => "Dusty",
            TrackSurface::Old => "Old",
            TrackSurface::Slow => "Slow",
            TrackSurface::Green => "Green",
            TrackSurface::Fast => "Fast",
            TrackSurface::Optimum => "Optimum",
        }
    }

    /// (grip % at session start, randomness, laps per +1% grip, % carried to
    /// the next session)
    fn parameters(self) -> (u32, u32, u32, u32) {
        match self {
            TrackSurface::Dusty => (86, 1, 30, 50),
            TrackSurface::Old => (89, 3, 50, 80),
            TrackSurface::Slow => (96, 1, 300, 80),
            TrackSurface::Green => (95, 2, 132, 90),
            TrackSurface::Fast => (98, 2, 700, 80),
            TrackSurface::Optimum => (100, 0, 1, 100),
        }
    }

    pub fn start_grip(self) -> u32 {
        self.parameters().0
    }

    pub fn summary(self) -> String {
        let (grip, _, lap_gain, _) = self.parameters();
        match self {
            TrackSurface::Optimum => format!("Grip {grip}%, no build-up"),
            _ => format!("Grip {grip}%, +1% every {lap_gain} laps"),
        }
    }
}

/// A driver aid that the car may or may not have in reality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aid {
    Off,
    /// As fitted to the real car.
    Factory,
    On,
}

impl Aid {
    pub fn label(self) -> &'static str {
        match self {
            Aid::Off => "Off",
            Aid::Factory => "Factory",
            Aid::On => "On",
        }
    }

    pub const ALL: [Aid; 3] = [Aid::Off, Aid::Factory, Aid::On];

    fn value(self) -> u32 {
        match self {
            Aid::Off => 0,
            Aid::Factory => 1,
            Aid::On => 2,
        }
    }

    fn of(value: f64) -> Self {
        match value as u32 {
            0 => Aid::Off,
            1 => Aid::Factory,
            _ => Aid::On,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assists {
    pub abs: Aid,
    pub traction_control: Aid,
    /// 0–100 %
    pub stability: u32,
    /// 0–100 %
    pub damage: u32,
    pub tyre_wear: u32,
    pub fuel_use: u32,
    pub ideal_line: bool,
    pub auto_clutch: bool,
    pub auto_shifter: bool,
    pub auto_blip: bool,
    pub tyre_blankets: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistPreset {
    Gamer,
    Racer,
    Pro,
}

impl AssistPreset {
    pub const ALL: [AssistPreset; 3] =
        [AssistPreset::Gamer, AssistPreset::Racer, AssistPreset::Pro];

    pub fn label(self) -> &'static str {
        match self {
            AssistPreset::Gamer => "Gamer",
            AssistPreset::Racer => "Racer",
            AssistPreset::Pro => "Pro",
        }
    }

    pub fn assists(self) -> Assists {
        match self {
            AssistPreset::Gamer => Assists {
                abs: Aid::On,
                traction_control: Aid::On,
                stability: 100,
                damage: 0,
                tyre_wear: 0,
                fuel_use: 0,
                ideal_line: true,
                auto_clutch: true,
                auto_shifter: true,
                auto_blip: true,
                tyre_blankets: true,
            },
            AssistPreset::Racer => Assists {
                abs: Aid::Factory,
                traction_control: Aid::Factory,
                stability: 0,
                damage: 50,
                tyre_wear: 1,
                fuel_use: 1,
                ideal_line: false,
                auto_clutch: true,
                auto_shifter: false,
                auto_blip: true,
                tyre_blankets: true,
            },
            AssistPreset::Pro => Assists {
                abs: Aid::Factory,
                traction_control: Aid::Factory,
                stability: 0,
                damage: 100,
                tyre_wear: 1,
                fuel_use: 1,
                ideal_line: false,
                auto_clutch: false,
                auto_shifter: false,
                auto_blip: false,
                tyre_blankets: true,
            },
        }
    }
}

impl Assists {
    pub fn preset(&self) -> Option<AssistPreset> {
        AssistPreset::ALL
            .into_iter()
            .find(|preset| preset.assists() == *self)
    }

    /// Reads `assists.ini`, taking `fallback`'s value for anything missing.
    pub fn from_ini(ini: &Ini, fallback: &Assists) -> Assists {
        let s = "ASSISTS";
        let number = |key| ini.get_number(s, key);
        let flag = |key, fallback| number(key).map_or(fallback, |n| n != 0.);
        let whole = |key, fallback| number(key).map_or(fallback, |n: f64| n.max(0.).round() as u32);
        Assists {
            abs: number("ABS").map_or(fallback.abs, Aid::of),
            traction_control: number("TRACTION_CONTROL").map_or(fallback.traction_control, Aid::of),
            stability: whole("STABILITY_CONTROL", fallback.stability).min(100),
            damage: whole("DAMAGE", fallback.damage).min(100),
            tyre_wear: whole("TYRE_WEAR", fallback.tyre_wear),
            fuel_use: whole("FUEL_RATE", fallback.fuel_use),
            ideal_line: flag("IDEAL_LINE", fallback.ideal_line),
            auto_clutch: flag("AUTO_CLUTCH", fallback.auto_clutch),
            auto_shifter: flag("AUTO_SHIFTER", fallback.auto_shifter),
            auto_blip: flag("AUTO_BLIP", fallback.auto_blip),
            tyre_blankets: flag("TYRE_BLANKETS", fallback.tyre_blankets),
        }
    }

    pub fn to_ini(&self) -> Ini {
        let mut ini = Ini::new();
        let s = "ASSISTS";
        ini.set(s, "IDEAL_LINE", u32::from(self.ideal_line))
            .set(s, "AUTO_BLIP", u32::from(self.auto_blip))
            .set(s, "STABILITY_CONTROL", self.stability)
            .set(s, "AUTO_BRAKE", 0)
            .set(s, "AUTO_SHIFTER", u32::from(self.auto_shifter))
            .set(s, "ABS", self.abs.value())
            .set(s, "TRACTION_CONTROL", self.traction_control.value())
            .set(s, "AUTO_CLUTCH", u32::from(self.auto_clutch))
            .set(s, "VISUALDAMAGE", u32::from(self.damage > 0))
            .set(s, "DAMAGE", self.damage)
            .set(s, "FUEL_RATE", self.fuel_use)
            .set(s, "TYRE_WEAR", self.tyre_wear)
            .set(s, "TYRE_BLANKETS", u32::from(self.tyre_blankets))
            .set(s, "SLIPSTREAM", 1);
        ini
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionConfig {
    pub mode: SessionMode,
    pub car_id: String,
    pub skin_id: String,
    pub track_id: String,
    pub layout_id: Option<String>,

    pub laps: u32,
    pub opponents: u32,
    /// 1-based grid slot.
    pub start_position: u32,
    pub grid: GridKind,
    /// 70–100 %
    pub ai_strength: u32,
    /// 0–100 %
    pub ai_aggression: u32,
    pub penalties: bool,
    /// Give each opponent a slightly different pace instead of one level.
    pub shuffle_ai_skill: bool,

    pub time_seconds: u32,
    pub time_multiplier: u32,
    pub weather_id: String,
    pub ambient_c: i32,
    pub wind_kmh: u32,
    pub wind_direction_deg: u32,
    pub surface: TrackSurface,

    pub assists: Assists,
    pub driver_name: String,
}

pub const TIME_MIN: u32 = 8 * 3600;
pub const TIME_MAX: u32 = 18 * 3600;

impl SessionConfig {
    /// A sensible first session for a fresh install: a short race in the
    /// first owned car at the first owned track.
    pub fn first_run(library: &Library) -> Option<Self> {
        let preferred = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        let car = preferred(&["ferrari_458_gt2", "bmw_z4_gt3", "abarth500"])
            .iter()
            .find_map(|id| library.car(id))
            .filter(|car| car.owned)
            .or_else(|| library.cars.iter().find(|car| car.owned))?;
        let track = preferred(&["spa", "mugello", "magione"])
            .iter()
            .find_map(|id| library.track(id))
            .filter(|track| track.owned)
            .or_else(|| library.tracks.iter().find(|track| track.owned))?;
        let weather = library
            .weather("5_light_clouds")
            .or_else(|| library.weather.first())
            .map(|w| w.id.clone())
            .unwrap_or_default();
        let pit_boxes = track.layouts[0].pit_boxes.unwrap_or(1);
        Some(SessionConfig {
            mode: SessionMode::Race,
            car_id: car.id.clone(),
            skin_id: car.skins.first().map(|s| s.id.clone()).unwrap_or_default(),
            track_id: track.id.clone(),
            layout_id: track.layouts[0].id.clone(),
            laps: 12,
            opponents: pit_boxes.saturating_sub(1).min(23),
            start_position: pit_boxes.min(8),
            grid: GridKind::SameClass,
            ai_strength: 96,
            ai_aggression: 40,
            penalties: true,
            shuffle_ai_skill: false,
            time_seconds: 14 * 3600 + 30 * 60,
            time_multiplier: 1,
            weather_id: weather,
            ambient_c: 22,
            wind_kmh: 8,
            wind_direction_deg: 45,
            surface: TrackSurface::Optimum,
            assists: AssistPreset::Pro.assists(),
            driver_name: String::new(),
        })
    }

    /// Opponents actually on track: none outside multi-car modes, and never
    /// more than the track has pit boxes for.
    pub fn opponent_count(&self, library: &Library) -> u32 {
        if !self.mode.has_opponents() {
            return 0;
        }
        self.opponents.min(self.max_opponents(library))
    }

    pub fn max_opponents(&self, library: &Library) -> u32 {
        library
            .track(&self.track_id)
            .and_then(|track| track.layout(self.layout_id.as_deref()).pit_boxes)
            .unwrap_or(1)
            .saturating_sub(1)
    }

    pub fn road_c(&self, library: &Library) -> i32 {
        let coefficient = library
            .weather(&self.weather_id)
            .map_or(0.0, |w| w.temperature_coefficient);
        road_temperature(f64::from(self.ambient_c), self.time_seconds, coefficient).round() as i32
    }

    /// Cars the opponents are drawn from, per [`SessionConfig::grid`].
    pub fn grid_pool<'a>(&self, library: &'a Library) -> Vec<&'a Car> {
        let Some(car) = library.car(&self.car_id) else {
            return Vec::new();
        };
        match self.grid {
            GridKind::SameCar => vec![car],
            GridKind::SameClass => {
                let pool = library.same_class(car);
                if pool.is_empty() { vec![car] } else { pool }
            }
        }
    }

    /// "Race · 12 laps · 24 cars", for the session summary bar.
    pub fn summary(&self, library: &Library) -> String {
        let mut parts = vec![self.mode.label().to_owned()];
        if self.mode.has_laps() {
            parts.push(plural(self.laps, "lap"));
        }
        let cars = self.opponent_count(library) + 1;
        if cars > 1 {
            parts.push(plural(cars, "car"));
        }
        parts.join(" · ")
    }

    pub fn race_ini(&self, library: &Library) -> Ini {
        let opponents = self.opponent_count(library);
        let mut ini = Ini::new();

        ini.set("RACE", "TRACK", &self.track_id)
            .set(
                "RACE",
                "CONFIG_TRACK",
                self.layout_id.as_deref().unwrap_or(""),
            )
            .set("RACE", "MODEL", &self.car_id)
            .set("RACE", "MODEL_CONFIG", "")
            .set("RACE", "SKIN", &self.skin_id)
            .set("RACE", "CARS", opponents + 1)
            .set("RACE", "AI_LEVEL", self.ai_strength)
            .set("RACE", "FIXED_SETUP", 0)
            .set("RACE", "PENALTIES", u32::from(self.penalties))
            .set(
                "RACE",
                "DRIFT_MODE",
                u32::from(self.mode == SessionMode::Drift),
            )
            .set(
                "RACE",
                "RACE_LAPS",
                if self.mode.has_laps() { self.laps } else { 0 },
            )
            .set("RACE", "JUMP_START_PENALTY", 0);
        ini.set("HEADER", "VERSION", 2);

        ini.set("CAR_0", "MODEL", "-")
            .set("CAR_0", "MODEL_CONFIG", "")
            .set("CAR_0", "SETUP", "")
            .set("CAR_0", "SKIN", &self.skin_id)
            .set("CAR_0", "DRIVER_NAME", &self.driver_name)
            .set("CAR_0", "NATIONALITY", "")
            .set("CAR_0", "BALLAST", 0)
            .set("CAR_0", "RESTRICTOR", 0);

        let pool = self.grid_pool(library);
        for slot in 1..=opponents {
            let section = format!("CAR_{slot}");
            let index = (slot - 1) as usize;
            let car = pool[index % pool.len()];
            // Walk each car's skins so identical cars do not share a livery
            // until every skin has been used once.
            let skin = car
                .skins
                .get((index / pool.len() + 1) % car.skins.len().max(1))
                .map_or("", |skin| skin.id.as_str());
            ini.set(&section, "MODEL", &car.id)
                .set(&section, "MODEL_CONFIG", "")
                .set(&section, "SETUP", "")
                .set(&section, "SKIN", skin)
                .set(&section, "DRIVER_NAME", format!("Driver {slot}"))
                .set(&section, "NATIONALITY", "")
                .set(&section, "AI_LEVEL", self.ai_level(slot, opponents))
                .set(&section, "AI_AGGRESSION", self.ai_aggression)
                .set(&section, "BALLAST", 0)
                .set(&section, "RESTRICTOR", 0);
        }

        for (index, session) in self.sessions().iter().enumerate() {
            let section = format!("SESSION_{index}");
            ini.set(&section, "NAME", session.name)
                .set(&section, "TYPE", session.kind)
                .set(&section, "SPAWN_SET", session.spawn_set);
            if session.kind == RACE {
                ini.set(&section, "LAPS", self.laps).set(
                    &section,
                    "STARTING_POSITION",
                    self.start_position.clamp(1, opponents + 1),
                );
            } else {
                ini.set(&section, "DURATION_MINUTES", session.minutes);
            }
        }

        ini.set("GHOST_CAR", "RECORDING", 0)
            .set("GHOST_CAR", "PLAYING", 0)
            .set("GHOST_CAR", "SECONDS_ADVANTAGE", 0)
            .set("GHOST_CAR", "LOAD", 0)
            .set("GHOST_CAR", "FILE", "");
        ini.set("REPLAY", "FILENAME", "").set("REPLAY", "ACTIVE", 0);

        ini.set("LIGHTING", "SUN_ANGLE", sun_angle(self.time_seconds))
            .set("LIGHTING", "TIME_MULT", self.time_multiplier)
            .set("LIGHTING", "CLOUD_SPEED", 0.2)
            // Custom Shaders Patch's Weather FX ignores the sun angle and
            // takes the time of day from a date, using the current time when
            // none is given. These are the keys Content Manager writes.
            .set(
                "LIGHTING",
                "__CM_UNCLAMPED_SUN_ANGLE",
                sun_angle(self.time_seconds),
            )
            .set(
                "LIGHTING",
                "__CM_DATE",
                patch_date(today(), self.time_seconds),
            )
            .set("LIGHTING", "__CM_DATE_USE_TIME", 1);

        let (grip, randomness, lap_gain, transfer) = self.surface.parameters();
        ini.set("GROOVE", "VIRTUAL_LAPS", 10)
            .set("GROOVE", "MAX_LAPS", 30)
            .set("GROOVE", "STARTING_LAPS", 0);
        ini.set("DYNAMIC_TRACK", "SESSION_START", grip)
            .set("DYNAMIC_TRACK", "SESSION_TRANSFER", transfer)
            .set("DYNAMIC_TRACK", "RANDOMNESS", randomness)
            .set("DYNAMIC_TRACK", "LAP_GAIN", lap_gain);

        ini.set("REMOTE", "ACTIVE", 0);
        ini.set(
            "LAP_INVALIDATOR",
            "ALLOWED_TYRES_OUT",
            if self.penalties { 2 } else { -1 },
        );
        ini.set("TEMPERATURE", "AMBIENT", self.ambient_c).set(
            "TEMPERATURE",
            "ROAD",
            self.road_c(library),
        );
        ini.set("WEATHER", "NAME", &self.weather_id);
        ini.set("WIND", "SPEED_KMH_MIN", self.wind_kmh)
            .set("WIND", "SPEED_KMH_MAX", self.wind_kmh)
            .set("WIND", "DIRECTION_DEG", self.wind_direction_deg);
        ini.set("BENCHMARK", "ACTIVE", 0);
        ini.set("RESTART", "ACTIVE", 0);
        ini.set("OPTIONS", "USE_MPH", 0);
        ini
    }

    fn ai_level(&self, slot: u32, opponents: u32) -> u32 {
        if !self.shuffle_ai_skill || opponents < 2 {
            return self.ai_strength;
        }
        // Spread the field over the six points below the chosen strength.
        let spread = 6 * (slot - 1) / (opponents - 1);
        self.ai_strength.saturating_sub(spread).max(70)
    }

    fn sessions(&self) -> Vec<Stage> {
        let stage = |name, kind, spawn_set, minutes| Stage {
            name,
            kind,
            spawn_set,
            minutes,
        };
        match self.mode {
            SessionMode::Practice => vec![stage("Practice", PRACTICE, "PIT", 0)],
            SessionMode::TrackDay => vec![stage("Track Day", PRACTICE, "PIT", 0)],
            SessionMode::Hotlap => vec![stage("Hotlap", HOTLAP, "HOTLAP_START", 0)],
            SessionMode::TimeAttack => vec![stage("Time Attack", TIME_ATTACK, "START", 0)],
            SessionMode::Drift => vec![stage("Drift", DRIFT, "PIT", 0)],
            SessionMode::Drag => vec![stage("Drag", DRAG, "START", 0)],
            SessionMode::Race => vec![stage("Quick Race", RACE, "START", 0)],
            SessionMode::Weekend => vec![
                stage("Practice", PRACTICE, "PIT", 10),
                stage("Qualifying", QUALIFY, "PIT", 15),
                stage("Race", RACE, "START", 0),
            ],
        }
    }
}

const PRACTICE: u32 = 1;
const QUALIFY: u32 = 2;
const RACE: u32 = 3;
const HOTLAP: u32 = 4;
const TIME_ATTACK: u32 = 5;
const DRIFT: u32 = 6;
const DRAG: u32 = 7;

struct Stage {
    name: &'static str,
    kind: u32,
    spawn_set: &'static str,
    minutes: u32,
}

/// The game expresses time of day as the sun's angle: 0° at 13:00, 16° per
/// hour, limited to the 08:00–18:00 range it can light.
pub fn sun_angle(seconds_since_midnight: u32) -> i32 {
    let seconds = seconds_since_midnight.clamp(TIME_MIN, TIME_MAX) as f64;
    (16.0 * (seconds - 13.0 * 3600.0) / 3600.0).round() as i32
}

/// Seconds since the Unix epoch at the start of today (UTC).
fn today() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    now - now % 86_400
}

/// The timestamp Custom Shaders Patch reads as `day` at the given time of
/// day. It treats the time of day as the track's local time.
fn patch_date(day: u64, seconds_since_midnight: u32) -> u64 {
    day + u64::from(seconds_since_midnight.clamp(TIME_MIN, TIME_MAX))
}

pub fn format_time(seconds_since_midnight: u32) -> String {
    format!(
        "{:02}:{:02}",
        seconds_since_midnight / 3600,
        seconds_since_midnight / 60 % 60
    )
}

/// "north-east" for 45°: where the wind blows from.
pub fn compass(degrees: u32) -> &'static str {
    const POINTS: [&str; 8] = [
        "north",
        "north-east",
        "east",
        "south-east",
        "south",
        "south-west",
        "west",
        "north-west",
    ];
    POINTS[((degrees % 360) as f64 / 45.0).round() as usize % 8]
}

fn plural(count: u32, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{CarSpecs, Origin, Skin, Track, TrackLayout, Weather};
    use std::path::PathBuf;

    fn car(id: &str, tags: &[&str], skins: &[&str]) -> Car {
        Car {
            id: id.to_owned(),
            dir: PathBuf::new(),
            name: id.to_owned(),
            brand: String::new(),
            class: "race".to_owned(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            country: None,
            year: None,
            author: None,
            version: None,
            description: String::new(),
            specs: CarSpecs::default(),
            power_curve: Vec::new(),
            torque_curve: Vec::new(),
            skins: skins
                .iter()
                .map(|id| Skin {
                    id: (*id).to_owned(),
                    name: (*id).to_owned(),
                    number: None,
                    team: None,
                    driver: None,
                    preview: None,
                    livery: None,
                })
                .collect(),
            origin: Origin::Kunos,
            owned: true,
        }
    }

    fn library() -> Library {
        Library {
            cars: vec![
                car("ferrari_458_gt2", &["gt2"], &["red", "yellow"]),
                car("bmw_m3_gt2", &["gt2"], &["white"]),
                car("bmw_z4_gt3", &["gt3"], &["blue"]),
            ],
            tracks: vec![Track {
                id: "spa".to_owned(),
                dir: PathBuf::new(),
                name: "Spa".to_owned(),
                layouts: vec![TrackLayout {
                    id: None,
                    full_name: "Spa".to_owned(),
                    name: "Spa".to_owned(),
                    description: String::new(),
                    country: None,
                    city: None,
                    length_m: Some(7004.0),
                    pit_boxes: Some(24),
                    direction: None,
                    tags: Vec::new(),
                    preview: None,
                    outline: None,
                }],
                origin: Origin::Kunos,
                owned: true,
            }],
            weather: vec![Weather {
                id: "3_clear".to_owned(),
                name: "Clear".to_owned(),
                temperature_coefficient: 1.0,
            }],
        }
    }

    fn config() -> SessionConfig {
        let mut config = SessionConfig::first_run(&library()).unwrap();
        config.weather_id = "3_clear".to_owned();
        config
    }

    #[test]
    fn first_run_is_a_race_in_the_preferred_car_and_track() {
        let config = config();
        assert_eq!(config.car_id, "ferrari_458_gt2");
        assert_eq!(config.skin_id, "red");
        assert_eq!(config.track_id, "spa");
        assert_eq!(config.opponents, 23);
        assert_eq!(config.summary(&library()), "Race · 12 laps · 24 cars");
    }

    #[test]
    fn race_fills_the_grid_from_the_same_class() {
        let ini = config().race_ini(&library());
        assert_eq!(ini.get("RACE", "CARS"), Some("24"));
        assert_eq!(ini.get("RACE", "RACE_LAPS"), Some("12"));
        assert_eq!(ini.get("CAR_0", "SKIN"), Some("red"));
        // Alternates between the two GT2 cars and never picks the GT3.
        assert_eq!(ini.get("CAR_1", "MODEL"), Some("ferrari_458_gt2"));
        assert_eq!(ini.get("CAR_2", "MODEL"), Some("bmw_m3_gt2"));
        assert_eq!(ini.get("CAR_23", "MODEL"), Some("ferrari_458_gt2"));
        assert_eq!(ini.get("CAR_24", "MODEL"), None);
        // The first opponent in the player's car takes the other skin.
        assert_eq!(ini.get("CAR_1", "SKIN"), Some("yellow"));
        assert_eq!(ini.get("SESSION_0", "TYPE"), Some("3"));
        assert_eq!(ini.get("SESSION_0", "STARTING_POSITION"), Some("8"));
        assert_eq!(ini.get("TEMPERATURE", "ROAD"), Some("31"));
        assert_eq!(ini.get("LIGHTING", "SUN_ANGLE"), Some("24"));
        let date: u64 = ini.get("LIGHTING", "__CM_DATE").unwrap().parse().unwrap();
        assert_eq!(date % 86_400, 14 * 3600 + 30 * 60);
        assert_eq!(ini.get("LIGHTING", "__CM_DATE_USE_TIME"), Some("1"));
    }

    #[test]
    fn solo_modes_have_no_opponents_and_no_lap_limit() {
        let mut config = config();
        config.mode = SessionMode::Hotlap;
        let ini = config.race_ini(&library());
        assert_eq!(ini.get("RACE", "CARS"), Some("1"));
        assert_eq!(ini.get("RACE", "RACE_LAPS"), Some("0"));
        assert_eq!(ini.get("CAR_1", "MODEL"), None);
        assert_eq!(ini.get("SESSION_0", "TYPE"), Some("4"));
        assert_eq!(config.summary(&library()), "Hotlap");
    }

    #[test]
    fn opponents_never_exceed_the_pit_boxes() {
        let mut config = config();
        config.opponents = 60;
        config.start_position = 40;
        let ini = config.race_ini(&library());
        assert_eq!(ini.get("RACE", "CARS"), Some("24"));
        assert_eq!(ini.get("SESSION_0", "STARTING_POSITION"), Some("24"));
    }

    #[test]
    fn weekend_runs_practice_qualifying_and_race() {
        let mut config = config();
        config.mode = SessionMode::Weekend;
        let ini = config.race_ini(&library());
        let kinds: Vec<_> = (0..3)
            .map(|i| ini.get(&format!("SESSION_{i}"), "TYPE").unwrap())
            .collect();
        assert_eq!(kinds, ["1", "2", "3"]);
    }

    #[test]
    fn shuffled_ai_spreads_below_the_chosen_strength() {
        let mut config = config();
        config.shuffle_ai_skill = true;
        let ini = config.race_ini(&library());
        assert_eq!(ini.get("CAR_1", "AI_LEVEL"), Some("96"));
        assert_eq!(ini.get("CAR_23", "AI_LEVEL"), Some("90"));
    }

    #[test]
    fn time_and_wind_are_described_in_words() {
        assert_eq!(format_time(14 * 3600 + 30 * 60), "14:30");
        assert_eq!(sun_angle(8 * 3600), -80);
        assert_eq!(sun_angle(23 * 3600), 80);
        // 2026-11-15, as Weather FX was seen to read it.
        assert_eq!(
            patch_date(1_794_700_800, 14 * 3600 + 30 * 60),
            1_794_753_000
        );
        assert_eq!(compass(45), "north-east");
        assert_eq!(compass(350), "north");
    }

    #[test]
    fn assists_read_back_from_their_file() {
        let racer = AssistPreset::Racer.assists();
        assert_eq!(
            Assists::from_ini(&racer.to_ini(), &AssistPreset::Pro.assists()),
            racer
        );
        let partial = Ini::parse("[ASSISTS]\nABS=0\nIDEAL_LINE=1\n");
        let read = Assists::from_ini(&partial, &AssistPreset::Pro.assists());
        assert_eq!(
            (read.abs, read.ideal_line, read.damage),
            (Aid::Off, true, 100)
        );
    }

    #[test]
    fn pro_assists_round_trip_to_their_preset() {
        let assists = AssistPreset::Pro.assists();
        assert_eq!(assists.preset(), Some(AssistPreset::Pro));
        let ini = assists.to_ini();
        assert_eq!(ini.get("ASSISTS", "ABS"), Some("1"));
        assert_eq!(ini.get("ASSISTS", "DAMAGE"), Some("100"));
        assert_eq!(ini.get("ASSISTS", "AUTO_CLUTCH"), Some("0"));
    }
}
