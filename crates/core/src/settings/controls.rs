//! `controls.ini`: which device drives, how its axes and buttons are bound,
//! and force feedback. Devices are numbered in `[CONTROLLERS]` (`CON0`,
//! `CON1`, …); a binding names a device by that number in `JOY`.

use std::path::{Path, PathBuf};

use crate::{Install, Result, ini::Ini, io_err, lenient};

use super::{CONTROLS, Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMethod {
    Wheel,
    Gamepad,
    Keyboard,
}

impl InputMethod {
    pub const ALL: [InputMethod; 3] = [
        InputMethod::Wheel,
        InputMethod::Gamepad,
        InputMethod::Keyboard,
    ];

    pub fn label(self) -> &'static str {
        match self {
            InputMethod::Wheel => "Wheel",
            InputMethod::Gamepad => "Gamepad",
            InputMethod::Keyboard => "Keyboard",
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            InputMethod::Wheel => "WHEEL",
            InputMethod::Gamepad => "X360",
            InputMethod::Keyboard => "KEYBOARD",
        }
    }

    pub fn of(ini: &Ini) -> Self {
        match ini.get("HEADER", "INPUT_METHOD") {
            Some("WHEEL") => InputMethod::Wheel,
            Some("X360") => InputMethod::Gamepad,
            _ => InputMethod::Keyboard,
        }
    }
}

/// The devices the bindings were made with, by their number.
pub fn devices(ini: &Ini) -> Vec<(u32, String)> {
    (0..32)
        .filter_map(|n| {
            let name = ini.get("CONTROLLERS", &format!("CON{n}"))?;
            (!name.is_empty()).then(|| (n, name.to_owned()))
        })
        .collect()
}

/// A wheel-and-pedals axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Steering,
    Throttle,
    Brake,
    Clutch,
    Handbrake,
}

impl Axis {
    pub const ALL: [Axis; 5] = [
        Axis::Steering,
        Axis::Throttle,
        Axis::Brake,
        Axis::Clutch,
        Axis::Handbrake,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Axis::Steering => "Steering",
            Axis::Throttle => "Throttle",
            Axis::Brake => "Brake",
            Axis::Clutch => "Clutch",
            Axis::Handbrake => "Handbrake",
        }
    }

    fn section(self) -> &'static str {
        match self {
            Axis::Steering => "STEER",
            Axis::Throttle => "THROTTLE",
            Axis::Brake => "BRAKES",
            Axis::Clutch => "CLUTCH",
            Axis::Handbrake => "HANDBRAKE",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisBinding {
    /// What it is bound to, e.g. "CSL DD · axis X"; `None` when unbound.
    pub bound_to: Option<String>,
    pub inverted: bool,
    /// The brake's response curve, when it has one.
    pub gamma: Option<f64>,
}

pub fn axis(ini: &Ini, axis: Axis) -> AxisBinding {
    let s = axis.section();
    let number = |key| ini.get_number(s, key);
    let joy = number("JOY").filter(|&n| n >= 0.);
    let index = number("AXLE").filter(|&n| n >= 0.);
    let bound_to = joy.zip(index).map(|(joy, index)| {
        let device = device_name(ini, joy as u32);
        format!("{device} · axis {}", axis_name(index as u32))
    });
    let inverted = match axis {
        Axis::Steering => number("SCALE").is_some_and(|scale| scale < 0.),
        _ => number("MIN").unwrap_or(-1.) > number("MAX").unwrap_or(1.),
    };
    AxisBinding {
        bound_to,
        inverted,
        gamma: (axis == Axis::Brake).then(|| number("GAMMA")).flatten(),
    }
}

/// Reverses an axis. Pedals map their travel from `MIN` to `MAX`, so
/// swapping the two turns them around; steering has its own sign.
pub fn set_inverted(config: &mut Config, axis: Axis, inverted: bool) -> Result<()> {
    let s = axis.section();
    config.edit(CONTROLS, |ini| {
        if axis == Axis::Steering {
            let scale = ini.get_number(s, "SCALE").unwrap_or(1.).abs();
            let scale = if inverted { -scale } else { scale };
            ini.set(s, "SCALE", scale);
            return;
        }
        let min = ini.get_number(s, "MIN").unwrap_or(-1.);
        let max = ini.get_number(s, "MAX").unwrap_or(1.);
        if (min > max) != inverted {
            ini.set(s, "MIN", max).set(s, "MAX", min);
        }
    })
}

fn device_name(ini: &Ini, number: u32) -> String {
    ini.get("CONTROLLERS", &format!("CON{number}"))
        .filter(|name| !name.is_empty())
        .map(short_device_name)
        .unwrap_or_else(|| format!("Device {}", number + 1))
}

/// Drops the maker's boilerplate from a device name.
fn short_device_name(name: &str) -> String {
    let mut name = name.trim().to_owned();
    for noise in [" Racing Wheel", " USB", " (TM)", "®", "™"] {
        name = name.replace(noise, "");
    }
    name
}

/// DirectInput's axes, in the order the game numbers them.
fn axis_name(index: u32) -> String {
    const NAMES: [&str; 8] = ["X", "Y", "Z", "Rx", "Ry", "Rz", "Slider 1", "Slider 2"];
    NAMES
        .get(index as usize)
        .map_or_else(|| (index + 1).to_string(), |name| (*name).to_owned())
}

/// Something the driver can do with a button.
pub struct Action {
    pub label: &'static str,
    section: &'static str,
    /// Keys under `[KEYBOARD]` and `[X360]` that older files use instead of
    /// the action's own section.
    keyboard_key: Option<&'static str>,
    gamepad_key: Option<&'static str>,
}

const fn action(label: &'static str, section: &'static str) -> Action {
    Action {
        label,
        section,
        keyboard_key: None,
        gamepad_key: None,
    }
}

pub const ACTIONS: &[Action] = &[
    Action {
        keyboard_key: Some("GEAR_UP"),
        gamepad_key: Some("GEAR_UP"),
        ..action("Shift up", "GEARUP")
    },
    Action {
        keyboard_key: Some("GEAR_DOWN"),
        gamepad_key: Some("GEAR_DN"),
        ..action("Shift down", "GEARDN")
    },
    Action {
        keyboard_key: Some("HANDBRAKE"),
        gamepad_key: Some("HANDBRAKE"),
        ..action("Handbrake", "HANDBRAKE")
    },
    action("Traction control +", "TCUP"),
    action("Traction control −", "TCDN"),
    action("ABS +", "ABSUP"),
    action("ABS −", "ABSDN"),
    action("Brake balance forward", "BALANCEUP"),
    action("Brake balance rearward", "BALANCEDN"),
    action("Turbo +", "TURBOUP"),
    action("Turbo −", "TURBODN"),
    action("Engine brake +", "ENGINE_BRAKE_UP"),
    action("Engine brake −", "ENGINE_BRAKE_DN"),
    action("KERS", "KERS"),
    action("DRS", "DRS"),
    action("Starter", "STARTER"),
    action("Headlights", "ACTION_HEADLIGHTS"),
    action("Flash headlights", "ACTION_HEADLIGHTS_FLASH"),
    action("Horn", "ACTION_HORN"),
    action("Change camera", "ACTION_CHANGE_CAMERA"),
    action("Look left", "GLANCELEFT"),
    action("Look right", "GLANCERIGHT"),
    action("Look back", "GLANCEBACK"),
];

/// What `action` is bound to for `method`, if anything.
pub fn binding(ini: &Ini, action: &Action, method: InputMethod) -> Option<String> {
    let s = action.section;
    match method {
        InputMethod::Wheel => {
            let joy = ini.get_number(s, "JOY").filter(|&n| n >= 0.)?;
            let button = ini.get_number(s, "BUTTON").filter(|&n| n >= 0.)?;
            let devices = devices(ini).len();
            let button = format!("Button {}", button as u32 + 1);
            // Name the device only when there is more than one to tell apart.
            Some(if devices > 1 {
                format!("{} · {button}", device_name(ini, joy as u32))
            } else {
                button
            })
        }
        InputMethod::Gamepad => ini
            .get(s, "XBOXBUTTON")
            .or_else(|| ini.get("X360", action.gamepad_key?))
            .and_then(gamepad_button),
        InputMethod::Keyboard => ini
            .get(s, "KEY")
            .and_then(key_name)
            .or_else(|| ini.get("KEYBOARD", action.keyboard_key?).and_then(key_name)),
    }
}

fn gamepad_button(value: &str) -> Option<String> {
    let name = match value.trim() {
        "" | "-1" => return None,
        "LSHOULDER" => "LB",
        "RSHOULDER" => "RB",
        "LTHUMB" => "Left stick",
        "RTHUMB" => "Right stick",
        "DPAD_UP" => "D-pad up",
        "DPAD_DOWN" => "D-pad down",
        "DPAD_LEFT" => "D-pad left",
        "DPAD_RIGHT" => "D-pad right",
        "BACK" => "Back",
        "START" => "Start",
        other => other,
    };
    Some(name.to_owned())
}

/// The key for a Windows virtual-key code such as `0x26`.
pub fn key_name(value: &str) -> Option<String> {
    let code = u32::from_str_radix(value.trim().strip_prefix("0x")?, 16).ok()?;
    let name = match code {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x10 | 0xA0 => "Shift",
        0xA1 => "Right Shift",
        0x11 | 0xA2 => "Ctrl",
        0xA3 => "Right Ctrl",
        0x12 | 0xA4 => "Alt",
        0xA5 => "Right Alt",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "←",
        0x26 => "↑",
        0x27 => "→",
        0x28 => "↓",
        0x2D => "Insert",
        0x2E => "Delete",
        0x30..=0x39 | 0x41..=0x5A => return char::from_u32(code).map(String::from),
        0x60..=0x69 => return Some(format!("Num {}", code - 0x60)),
        0x70..=0x87 => return Some(format!("F{}", code - 0x6F)),
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        _ => return Some(format!("Key {code:#04X}")),
    };
    Some(name.to_owned())
}

/// A saved set of controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub path: PathBuf,
    /// Saved by the player rather than shipped with the game.
    pub own: bool,
}

/// Where the player's own presets go, as the stock launcher keeps them.
pub fn presets_dir(install: &Install) -> PathBuf {
    install.cfg_dir().join("controllers/savedsetups")
}

/// The player's presets first, then the game's.
pub fn presets(install: &Install) -> Vec<Preset> {
    let mut list = Vec::new();
    for (dir, own) in [
        (presets_dir(install), true),
        (install.root.join("cfg/controllers/presets"), false),
    ] {
        let mut found: Vec<Preset> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = path
                    .file_name()?
                    .to_str()?
                    .strip_suffix(".ini")?
                    .replace('_', " ");
                Some(Preset { name, path, own })
            })
            .collect();
        found.sort_by_key(|preset| preset.name.to_lowercase());
        list.extend(found);
    }
    list
}

/// Makes a preset the controls in use.
pub fn load_preset(config: &mut Config, preset: &Path) -> Result<()> {
    let bytes = std::fs::read(preset).map_err(io_err(preset))?;
    config.replace(CONTROLS, Ini::parse(&lenient::decode(&bytes)))
}

/// Saves the controls in use as a preset of the player's own.
pub fn save_preset(config: &mut Config, install: &Install, name: &str) -> Result<PathBuf> {
    let file: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let file = if file.is_empty() {
        "Preset".to_owned()
    } else {
        file
    };
    let dir = presets_dir(install);
    std::fs::create_dir_all(&dir).map_err(io_err(&dir))?;
    let path = dir.join(format!("{file}.ini"));
    let text = config.file(CONTROLS).to_string();
    crate::launch::write(&path, &text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G25: &str = "[HEADER]\nINPUT_METHOD=WHEEL\n[CONTROLLERS]\nCON0=Logitech G25 Racing Wheel USB\n\
        [STEER]\nJOY=0\nAXLE=0\nSCALE=1\n[THROTTLE]\nJOY=0\nAXLE=1\nMIN=1\nMAX=-1\n\
        [BRAKES]\nJOY=0\nAXLE=5\nMIN=-1\nMAX=1\nGAMMA=2.4\n[CLUTCH]\nJOY=-1\nAXLE=-1\n\
        [GEARUP]\nJOY=0\nBUTTON=4\nKEY=0x20\nXBOXBUTTON=A\n[GEARDN]\nJOY=-1\nBUTTON=-1\nKEY=0xA2\nXBOXBUTTON=X\n";

    #[test]
    fn axes_name_their_device_and_direction() {
        let ini = Ini::parse(G25);
        assert_eq!(InputMethod::of(&ini), InputMethod::Wheel);
        let steering = axis(&ini, Axis::Steering);
        assert_eq!(steering.bound_to.as_deref(), Some("Logitech G25 · axis X"));
        assert!(!steering.inverted);
        assert!(axis(&ini, Axis::Throttle).inverted);
        let brake = axis(&ini, Axis::Brake);
        assert_eq!(brake.bound_to.as_deref(), Some("Logitech G25 · axis Rz"));
        assert_eq!(brake.gamma, Some(2.4));
        assert_eq!(axis(&ini, Axis::Clutch).bound_to, None);
    }

    #[test]
    fn buttons_read_per_input_method() {
        let ini = Ini::parse(G25);
        let [up, down] = [&ACTIONS[0], &ACTIONS[1]];
        assert_eq!(
            binding(&ini, up, InputMethod::Wheel).as_deref(),
            Some("Button 5")
        );
        assert_eq!(binding(&ini, down, InputMethod::Wheel), None);
        assert_eq!(
            binding(&ini, up, InputMethod::Keyboard).as_deref(),
            Some("Space")
        );
        assert_eq!(
            binding(&ini, down, InputMethod::Keyboard).as_deref(),
            Some("Ctrl")
        );
        assert_eq!(
            binding(&ini, up, InputMethod::Gamepad).as_deref(),
            Some("A")
        );
    }

    #[test]
    fn inverting_swaps_pedal_ends_and_flips_steering() {
        let base = std::env::temp_dir().join(format!("muretto-controls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("steamapps/common/assettocorsa");
        std::fs::create_dir_all(root.join("cfg")).unwrap();
        std::fs::write(root.join("cfg/controls.ini"), G25).unwrap();
        let install = Install::at(root, None);
        let mut config = Config::load(&install);

        set_inverted(&mut config, Axis::Throttle, false).unwrap();
        set_inverted(&mut config, Axis::Brake, true).unwrap();
        set_inverted(&mut config, Axis::Steering, true).unwrap();
        let ini = config.file(CONTROLS).clone();
        assert_eq!(ini.get("THROTTLE", "MIN"), Some("-1"));
        assert_eq!(ini.get("BRAKES", "MIN"), Some("1"));
        assert_eq!(ini.get("STEER", "SCALE"), Some("-1"));
        // Asking for what is already so changes nothing.
        set_inverted(&mut config, Axis::Brake, true).unwrap();
        assert_eq!(config.file(CONTROLS).get("BRAKES", "MIN"), Some("1"));

        let saved = save_preset(&mut config, &install, "My: wheel").unwrap();
        assert_eq!(saved.file_name().unwrap(), "My_ wheel.ini");
        let listed = presets(&install);
        assert_eq!(listed[0].name, "My  wheel");
        assert!(listed[0].own);
        set_inverted(&mut config, Axis::Brake, false).unwrap();
        load_preset(&mut config, &saved).unwrap();
        assert_eq!(config.file(CONTROLS).get("BRAKES", "MIN"), Some("1"));
    }

    #[test]
    fn virtual_keys_have_names() {
        assert_eq!(key_name("0x41").as_deref(), Some("A"));
        assert_eq!(key_name("0x72").as_deref(), Some("F3"));
        assert_eq!(key_name("-1"), None);
        assert_eq!(key_name(""), None);
    }
}
