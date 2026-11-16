//! A preset, and the host's own settings, as the files AssettoServer reads
//! from `cfg/`.
//!
//! Text in a preset has been through `Preset::validate`, so nothing in it
//! breaks out of its line.

use std::fmt::{Display, Write as _};
use std::io;
use std::path::Path;

use muretto_host_protocol::presets::{AiMode, AidRule, Preset, RaceJoin, RaceLength, StartRule};

use crate::config::Host;
use crate::store::write_atomic;

/// The files of `cfg/` the agent owns. Others, such as plugin settings put
/// there by hand, are left alone.
#[derive(Debug, Clone, PartialEq)]
pub struct CfgFiles {
    pub server_cfg: String,
    pub entry_list: String,
    pub extra_cfg: Option<String>,
    pub csp_extra_options: Option<String>,
    pub welcome: Option<String>,
}

const WELCOME: &str = "welcome.txt";

pub fn render(preset: &Preset, host: &Host) -> CfgFiles {
    CfgFiles {
        server_cfg: server_cfg(preset, host),
        entry_list: entry_list(preset),
        extra_cfg: preset.extra_cfg_yml.clone(),
        csp_extra_options: preset.csp_extra_options.clone(),
        welcome: preset.access.welcome_message.clone(),
    }
}

impl CfgFiles {
    /// Writes the files into `cfg`, each in one step, and removes the
    /// optional ones the preset does not set: AssettoServer writes its own
    /// `extra_cfg.yml` when there is none.
    pub fn write(&self, cfg: &Path) -> io::Result<()> {
        std::fs::create_dir_all(cfg)?;
        write_atomic(&cfg.join("server_cfg.ini"), self.server_cfg.as_bytes())?;
        write_atomic(&cfg.join("entry_list.ini"), self.entry_list.as_bytes())?;
        for (name, content) in [
            ("extra_cfg.yml", &self.extra_cfg),
            ("csp_extra_options.ini", &self.csp_extra_options),
            (WELCOME, &self.welcome),
        ] {
            let path = cfg.join(name);
            match content {
                Some(content) => write_atomic(&path, content.as_bytes())?,
                None => match std::fs::remove_file(&path) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                    _ => {}
                },
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Ini(String);

impl Ini {
    fn section(&mut self, name: impl Display) {
        if !self.0.is_empty() {
            self.0.push('\n');
        }
        let _ = writeln!(self.0, "[{name}]");
    }

    fn set(&mut self, key: &str, value: impl Display) {
        let _ = writeln!(self.0, "{key}={value}");
    }
}

fn server_cfg(preset: &Preset, host: &Host) -> String {
    let mut ini = Ini::default();
    let rules = &preset.rules;
    let sessions = &preset.sessions;
    let conditions = &preset.conditions;

    ini.section("SERVER");
    let name = match host.name.trim() {
        "" => preset.name.clone(),
        host => format!("{host} | {}", preset.name),
    };
    ini.set("NAME", name);
    ini.set("CARS", preset.cars().join(";"));
    ini.set("TRACK", &preset.track.id);
    ini.set("CONFIG_TRACK", preset.track.layout.as_deref().unwrap_or(""));
    ini.set("SUN_ANGLE", sun_angle(conditions.time_of_day));
    ini.set("TIME_OF_DAY_MULT", conditions.time_multiplier);
    ini.set("PASSWORD", preset.access.password.as_deref().unwrap_or(""));
    ini.set(
        "ADMIN_PASSWORD",
        host.admin_password.as_deref().unwrap_or(""),
    );
    ini.set("TCP_PORT", host.tcp_port);
    ini.set("UDP_PORT", host.udp_port);
    ini.set("HTTP_PORT", host.http_port);
    ini.set("REGISTER_TO_LOBBY", flag(host.register_to_lobby));
    let slots = preset.entries.len().min(255) as u8;
    let mut max_clients = preset.access.max_clients.unwrap_or(slots).min(slots);
    if let Some(cap) = host.max_clients {
        max_clients = max_clients.min(cap);
    }
    ini.set("MAX_CLIENTS", max_clients);
    ini.set("PICKUP_MODE_ENABLED", flag(preset.access.pickup_mode));
    ini.set("LOCKED_ENTRY_LIST", flag(preset.access.locked_entry_list));
    ini.set(
        "WELCOME_MESSAGE",
        if preset.access.welcome_message.is_some() {
            format!("cfg/{WELCOME}")
        } else {
            String::new()
        },
    );
    ini.set("LOOP_MODE", flag(sessions.loop_mode));
    ini.set("RESULT_SCREEN_TIME", sessions.result_screen_secs);
    ini.set("ABS_ALLOWED", aid(rules.abs));
    ini.set("TC_ALLOWED", aid(rules.traction_control));
    ini.set("STABILITY_ALLOWED", flag(rules.stability_control));
    ini.set("AUTOCLUTCH_ALLOWED", flag(rules.autoclutch));
    ini.set("TYRE_BLANKETS_ALLOWED", flag(rules.tyre_blankets));
    ini.set("FORCE_VIRTUAL_MIRROR", flag(rules.force_virtual_mirror));
    ini.set("FUEL_RATE", rules.fuel_rate);
    ini.set("DAMAGE_MULTIPLIER", rules.damage);
    ini.set("TYRE_WEAR_RATE", rules.tyre_wear);
    ini.set("ALLOWED_TYRES_OUT", rules.tyres_out_allowed);
    ini.set("LEGAL_TYRES", rules.legal_tyres.join(";"));
    ini.set(
        "MAX_CONTACTS_PER_KM",
        rules.max_contacts_per_km.map_or(-1, i16::from),
    );
    ini.set("KICK_QUORUM", rules.kick_quorum);
    ini.set("VOTING_QUORUM", rules.vote_quorum);
    ini.set("VOTE_DURATION", rules.vote_secs);
    ini.set(
        "START_RULE",
        match rules.start_rule {
            StartRule::CarLocked => 0,
            StartRule::TeleportToPits => 1,
            StartRule::DriveThrough => 2,
        },
    );
    if let Some(race) = &sessions.race {
        ini.set("RACE_OVER_TIME", race.over_secs);
        let (opens, closes) = race
            .pit_window
            .map_or((0, 0), |window| (window.opens_lap, window.closes_lap));
        ini.set("RACE_PIT_WINDOW_START", opens);
        ini.set("RACE_PIT_WINDOW_END", closes);
        ini.set("REVERSED_GRID_RACE_POSITIONS", race.reversed_grid);
        let extra_lap = matches!(
            race.length,
            RaceLength::Minutes {
                extra_lap: true,
                ..
            }
        );
        ini.set("RACE_EXTRA_LAP", flag(extra_lap));
    }

    if let Some(booking) = sessions.booking {
        ini.section("BOOKING");
        ini.set("NAME", "Booking");
        ini.set("TIME", booking.minutes);
    }
    for (section, name, session) in [
        ("PRACTICE", "Practice", sessions.practice),
        ("QUALIFY", "Qualify", sessions.qualify),
    ] {
        if let Some(session) = session {
            ini.section(section);
            ini.set("NAME", name);
            ini.set("TIME", session.minutes);
            ini.set("IS_OPEN", flag(session.open));
        }
    }
    if let Some(race) = &sessions.race {
        ini.section("RACE");
        ini.set("NAME", "Race");
        let (laps, minutes) = match race.length {
            RaceLength::Laps { laps } => (laps, 0),
            RaceLength::Minutes { minutes, .. } => (0, minutes),
        };
        ini.set("LAPS", laps);
        ini.set("TIME", minutes);
        ini.set("WAIT_TIME", race.wait_secs);
        ini.set(
            "IS_OPEN",
            match race.join {
                RaceJoin::Closed => 0,
                RaceJoin::Open => 1,
                RaceJoin::BeforeStart => 2,
            },
        );
    }

    let grip = conditions.grip;
    ini.section("DYNAMIC_TRACK");
    ini.set("SESSION_START", grip.start_percent);
    ini.set("RANDOMNESS", grip.randomness);
    ini.set("SESSION_TRANSFER", grip.transfer_percent);
    ini.set("LAP_GAIN", grip.lap_gain);

    for (i, weather) in conditions.weather.iter().enumerate() {
        ini.section(format_args!("WEATHER_{i}"));
        ini.set("GRAPHICS", &weather.graphics);
        ini.set("BASE_TEMPERATURE_AMBIENT", weather.ambient_c);
        ini.set("BASE_TEMPERATURE_ROAD", weather.road_offset_c);
        ini.set("VARIATION_AMBIENT", weather.ambient_variation_c);
        ini.set("VARIATION_ROAD", weather.road_variation_c);
        ini.set("WIND_BASE_SPEED_MIN", weather.wind_min_kmh);
        ini.set("WIND_BASE_SPEED_MAX", weather.wind_max_kmh);
        ini.set("WIND_BASE_DIRECTION", weather.wind_direction_deg);
        ini.set(
            "WIND_VARIATION_DIRECTION",
            weather.wind_direction_variation_deg,
        );
    }
    ini.0
}

fn entry_list(preset: &Preset) -> String {
    let mut ini = Ini::default();
    for (i, entry) in preset.entries.iter().enumerate() {
        ini.section(format_args!("CAR_{i}"));
        ini.set("MODEL", &entry.model);
        ini.set("SKIN", entry.skin.as_deref().unwrap_or(""));
        ini.set("SPECTATOR_MODE", 0);
        ini.set("DRIVERNAME", entry.driver_name.as_deref().unwrap_or(""));
        ini.set("TEAM", entry.team.as_deref().unwrap_or(""));
        ini.set("GUID", entry.guid.as_deref().unwrap_or(""));
        ini.set("BALLAST", entry.ballast_kg);
        ini.set("RESTRICTOR", entry.restrictor);
        match entry.ai {
            AiMode::None => {}
            AiMode::Auto => ini.set("AI", "auto"),
            AiMode::Fixed => ini.set("AI", "fixed"),
        }
    }
    ini.0
}

fn flag(on: bool) -> u8 {
    u8::from(on)
}

fn aid(rule: AidRule) -> u8 {
    match rule {
        AidRule::Denied => 0,
        AidRule::Factory => 1,
        AidRule::Forced => 2,
    }
}

/// The game expresses time of day as the sun's angle: 0° at 13:00, 16° per
/// hour, limited to the 08:00–18:00 range it can light.
fn sun_angle(seconds_since_midnight: u32) -> i32 {
    let seconds = seconds_since_midnight.clamp(8 * 3600, 18 * 3600) as f64;
    (16.0 * (seconds - 13.0 * 3600.0) / 3600.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use muretto_host_protocol::presets::{Entry, Race, TrackRef};

    fn entry(model: &str, ai: AiMode) -> Entry {
        Entry {
            model: model.into(),
            skin: Some("red".into()),
            driver_name: None,
            team: None,
            guid: None,
            ballast_kg: 10,
            restrictor: 0,
            ai,
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
                entry("ks_audi_r8_lms", AiMode::None),
                entry("ks_porsche_911_gt3_r_2016", AiMode::Auto),
                entry("ks_audi_r8_lms", AiMode::None),
            ],
        )
    }

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn the_host_keeps_its_ports_and_names_the_server() {
        let host = Host {
            name: "Stefan's box".into(),
            admin_password: Some("secret".into()),
            max_clients: Some(2),
            ..Host::default()
        };
        let cfg = server_cfg(&preset(), &host);
        let lines = lines(&cfg);
        for line in [
            "NAME=Stefan's box | Sunday GT3",
            "CARS=ks_audi_r8_lms;ks_porsche_911_gt3_r_2016",
            "TRACK=ks_brands_hatch",
            "CONFIG_TRACK=gp",
            "ADMIN_PASSWORD=secret",
            "TCP_PORT=9600",
            "HTTP_PORT=8081",
            "MAX_CLIENTS=2",
            "SUN_ANGLE=16",
            "MAX_CONTACTS_PER_KM=-1",
            "WELCOME_MESSAGE=",
        ] {
            assert!(lines.contains(&line), "{line} missing from\n{cfg}");
        }
        assert!(!cfg.contains("[RACE]"));
    }

    #[test]
    fn sessions_and_weather_get_their_sections() {
        let mut preset = preset();
        preset.sessions.race = Some(Race {
            length: RaceLength::Minutes {
                minutes: 20,
                extra_lap: true,
            },
            join: RaceJoin::BeforeStart,
            wait_secs: 60,
            over_secs: 120,
            pit_window: None,
            reversed_grid: 0,
        });
        let cfg = server_cfg(&preset, &Host::default());
        assert!(cfg.contains("\n[PRACTICE]\nNAME=Practice\nTIME=30\nIS_OPEN=1\n"));
        assert!(cfg.contains("\n[RACE]\nNAME=Race\nLAPS=0\nTIME=20\nWAIT_TIME=60\nIS_OPEN=2\n"));
        assert!(cfg.contains("RACE_EXTRA_LAP=1\n"));
        assert!(cfg.contains("\n[WEATHER_0]\nGRAPHICS=3_clear\n"));
        assert!(cfg.contains("\n[DYNAMIC_TRACK]\nSESSION_START=98\n"));
    }

    #[test]
    fn entries_become_numbered_cars() {
        let list = entry_list(&preset());
        assert!(list.starts_with("[CAR_0]\nMODEL=ks_audi_r8_lms\nSKIN=red\n"));
        assert!(list.contains("[CAR_1]\nMODEL=ks_porsche_911_gt3_r_2016\n"));
        assert_eq!(list.matches("AI=auto").count(), 1);
        assert!(list.contains("[CAR_2]"));
    }

    #[test]
    fn the_sun_follows_the_clock() {
        assert_eq!(sun_angle(13 * 3600), 0);
        assert_eq!(sun_angle(8 * 3600), -80);
        assert_eq!(sun_angle(3 * 3600), -80);
        assert_eq!(sun_angle(18 * 3600), 80);
    }
}
