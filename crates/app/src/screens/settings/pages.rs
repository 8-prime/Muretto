//! What each Assetto Corsa settings page offers, and where the game keeps it.

use muretto_core::{
    session::Aid,
    settings::{AUDIO, CAMERA, CONTROLS, Config, GAMEPLAY, PYTHON, REPLAY, VIDEO},
};

use super::form::*;
use crate::game::Game;

/// A titled group of settings, in the left or right column.
pub struct Section {
    pub title: &'static str,
    pub right: bool,
    pub defs: &'static [Def],
}

/// Panels a page shows besides its sections.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Extra {
    None,
    /// Gamer, Racer and Pro, above the sections.
    AssistPresets,
    /// The installed Python apps, each with a switch.
    Apps,
    /// Where the files are, and putting them all back.
    Files,
}

pub struct Page {
    pub name: &'static str,
    pub blurb: &'static str,
    /// What "Restore defaults" puts back.
    pub files: &'static [&'static str],
    pub sections: &'static [Section],
    pub extra: Extra,
}

pub const VIDEO_PAGE: Page = Page {
    name: "Video",
    blurb: "Screen, image quality and effects.",
    files: &[VIDEO, CAMERA],
    extra: Extra::None,
    sections: &[
        Section {
            title: "Display",
            right: false,
            defs: &[
                found("Resolution", Key::Resolution, resolutions, "1920x1080"),
                choice(
                    "Refresh rate",
                    ini(VIDEO, "VIDEO", "REFRESH"),
                    &[
                        ("60", "60 Hz"),
                        ("75", "75 Hz"),
                        ("100", "100 Hz"),
                        ("120", "120 Hz"),
                        ("144", "144 Hz"),
                        ("165", "165 Hz"),
                        ("240", "240 Hz"),
                    ],
                    "60",
                ),
                choice(
                    "Display mode",
                    ini(VIDEO, "VIDEO", "FULLSCREEN"),
                    &[("1", "Fullscreen"), ("0", "Windowed")],
                    "1",
                ),
                choice(
                    "Rendering",
                    ini(VIDEO, "CAMERA", "MODE"),
                    &[
                        ("DEFAULT", "Single screen"),
                        ("TRIPLE", "Triple screen"),
                        ("OCULUS", "Oculus Rift"),
                        ("OPENVR", "OpenVR"),
                    ],
                    "DEFAULT",
                ),
                choice(
                    "Frame rate limit",
                    Key::FpsLimit,
                    &[
                        ("0", "No limit"),
                        ("30", "30 fps"),
                        ("60", "60 fps"),
                        ("75", "75 fps"),
                        ("90", "90 fps"),
                        ("120", "120 fps"),
                        ("144", "144 fps"),
                        ("165", "165 fps"),
                        ("240", "240 fps"),
                    ],
                    "0",
                ),
                choice(
                    "Anti-aliasing",
                    ini(VIDEO, "VIDEO", "AASAMPLES"),
                    &[("1", "Off"), ("2", "2×"), ("4", "4×"), ("8", "8×")],
                    "2",
                ),
                toggle("Vertical sync", ini(VIDEO, "VIDEO", "VSYNC"), false)
                    .note("Stops tearing at the cost of a little input delay."),
            ],
        },
        Section {
            title: "Quality",
            right: false,
            defs: &[
                choice(
                    "Texture filtering",
                    ini(VIDEO, "VIDEO", "ANISOTROPIC"),
                    &[
                        ("0", "Off"),
                        ("2", "2×"),
                        ("4", "4×"),
                        ("8", "8×"),
                        ("16", "16×"),
                    ],
                    "8",
                ),
                choice(
                    "Shadow resolution",
                    ini(VIDEO, "VIDEO", "SHADOW_MAP_SIZE"),
                    &[
                        ("512", "512"),
                        ("1024", "1024"),
                        ("2048", "2048"),
                        ("4096", "4096"),
                    ],
                    "2048",
                ),
                choice(
                    "World detail",
                    ini(VIDEO, "ASSETTOCORSA", "WORLD_DETAIL"),
                    &[
                        ("0", "Minimum"),
                        ("1", "Low"),
                        ("2", "Medium"),
                        ("3", "High"),
                        ("4", "Very high"),
                        ("5", "Maximum"),
                    ],
                    "5",
                ),
                choice(
                    "Smoke",
                    ini(VIDEO, "EFFECTS", "SMOKE"),
                    &[("0", "Off"), ("1", "Low"), ("2", "Medium"), ("3", "High")],
                    "3",
                ),
                range(
                    "Motion blur",
                    ini(VIDEO, "EFFECTS", "MOTION_BLUR"),
                    (0., 12., 1.),
                    1.,
                    whole_or_off,
                    "0",
                ),
                range(
                    "Colour saturation",
                    ini(VIDEO, "SATURATION", "LEVEL"),
                    (0., 200., 5.),
                    1.,
                    percent,
                    "100",
                ),
                toggle(
                    "Smoke in mirrors",
                    ini(VIDEO, "EFFECTS", "RENDER_SMOKE_IN_MIRROR"),
                    true,
                ),
            ],
        },
        Section {
            title: "Reflections and mirrors",
            right: true,
            defs: &[
                choice(
                    "Reflection resolution",
                    ini(VIDEO, "CUBEMAP", "SIZE"),
                    &[
                        ("256", "256"),
                        ("512", "512"),
                        ("1024", "1024"),
                        ("2048", "2048"),
                    ],
                    "512",
                ),
                choice(
                    "Mirror resolution",
                    ini(VIDEO, "MIRROR", "SIZE"),
                    &[
                        ("256", "256"),
                        ("512", "512"),
                        ("1024", "1024"),
                        ("2048", "2048"),
                    ],
                    "512",
                ),
                range(
                    "Reflection updates",
                    ini(VIDEO, "CUBEMAP", "FACES_PER_FRAME"),
                    (0., 6., 1.),
                    1.,
                    faces,
                    "0",
                ),
                range(
                    "Reflection distance",
                    ini(VIDEO, "CUBEMAP", "FARPLANE"),
                    (100., 2500., 50.),
                    1.,
                    metres,
                    "500",
                ),
                toggle("High-quality mirrors", ini(VIDEO, "MIRROR", "HQ"), false),
            ],
        },
        Section {
            title: "Post-processing",
            right: true,
            defs: &[
                toggle(
                    "Post-processing",
                    ini(VIDEO, "POST_PROCESS", "ENABLED"),
                    true,
                )
                .note("The filter and effects below only apply while this is on."),
                found(
                    "Filter",
                    ini(VIDEO, "POST_PROCESS", "FILTER"),
                    filters,
                    "default",
                ),
                range(
                    "Effects quality",
                    ini(VIDEO, "POST_PROCESS", "QUALITY"),
                    (0., 5., 1.),
                    1.,
                    whole,
                    "3",
                ),
                range(
                    "Glare",
                    ini(VIDEO, "POST_PROCESS", "GLARE"),
                    (0., 5., 1.),
                    1.,
                    whole_or_off,
                    "3",
                ),
                range(
                    "Depth of field",
                    ini(VIDEO, "POST_PROCESS", "DOF"),
                    (0., 5., 1.),
                    1.,
                    whole_or_off,
                    "3",
                ),
                toggle("Sun rays", ini(VIDEO, "POST_PROCESS", "RAYS_OF_GOD"), true),
                toggle(
                    "Heat shimmer",
                    ini(VIDEO, "POST_PROCESS", "HEAT_SHIMMER"),
                    true,
                ),
                toggle("FXAA", ini(VIDEO, "POST_PROCESS", "FXAA"), true)
                    .note("Smooths edges cheaply. Has no effect in VR."),
            ],
        },
        Section {
            title: "Cockpit view",
            right: false,
            defs: &[
                range(
                    "Field of view",
                    ini(CAMERA, "MODE", "FOV"),
                    (10., 120., 1.),
                    1.,
                    degrees,
                    "56",
                ),
                toggle(
                    "Hide driver's arms",
                    ini(VIDEO, "ASSETTOCORSA", "HIDE_ARMS"),
                    false,
                ),
                toggle(
                    "Hide steering wheel",
                    ini(VIDEO, "ASSETTOCORSA", "HIDE_STEER"),
                    false,
                ),
                toggle(
                    "Lock steering wheel",
                    ini(VIDEO, "ASSETTOCORSA", "LOCK_STEER"),
                    false,
                )
                .note("The wheel on screen stays still while you steer."),
                toggle(
                    "Level horizon",
                    ini(CAMERA, "MODE", "IS_WORLD_ALIGNED"),
                    false,
                )
                .note("The camera stays level instead of leaning with the car."),
            ],
        },
    ],
};

pub const AUDIO_PAGE: Page = Page {
    name: "Audio",
    blurb: "How loud each part of the sound is.",
    files: &[AUDIO],
    extra: Extra::None,
    sections: &[
        Section {
            title: "Levels",
            right: false,
            defs: &[
                volume("Master", "MASTER", "1"),
                volume("Engine", "ENGINE", "1"),
                volume("Tyres", "TYRES", "0.5"),
                volume("Road surface", "SURFACES", "0.9"),
                volume("Bottoming out", "DIRT_BOTTOM", "1"),
                volume("Wind", "WIND", "0.4"),
            ],
        },
        Section {
            title: "Around you",
            right: true,
            defs: &[
                volume("Other cars", "OPPONENTS", "0.8"),
                volume("Interface", "UISOUNDS", "0.7"),
                volume("Music", "MUSIC", "0.5"),
            ],
        },
    ],
};

const fn volume(label: &'static str, key: &'static str, default: &'static str) -> Def {
    range(
        label,
        ini(AUDIO, "LEVELS", key),
        (0., 100., 1.),
        100.,
        percent,
        default,
    )
}

const AIDS: &[(&str, &str)] = &[("Off", "Off"), ("Factory", "Factory"), ("On", "On")];
const RATES: &[(&str, &str)] = &[
    ("0", "Off"),
    ("1", "×1"),
    ("2", "×2"),
    ("3", "×3"),
    ("4", "×4"),
    ("5", "×5"),
];

fn aid(value: &str) -> Aid {
    Aid::ALL
        .into_iter()
        .find(|aid| aid.label() == value)
        .unwrap_or(Aid::Factory)
}

fn flag(on: bool) -> String {
    if on { "1" } else { "0" }.to_owned()
}

fn is_on(value: &str) -> bool {
    value == "1"
}

fn amount(value: &str) -> u32 {
    value.parse::<f32>().map_or(0, |v| v.max(0.).round() as u32)
}

pub const ASSISTS_PAGE: Page = Page {
    name: "Assists and realism",
    blurb: "Help from the car and how forgiving the simulation is. Quick drive uses these for every session.",
    files: &[],
    extra: Extra::AssistPresets,
    sections: &[
        Section {
            title: "Driving aids",
            right: false,
            defs: &[
                choice(
                    "ABS",
                    Key::Assist {
                        get: |a| a.abs.label().to_owned(),
                        set: |a, v| a.abs = aid(v),
                    },
                    AIDS,
                    "Factory",
                ),
                choice(
                    "Traction control",
                    Key::Assist {
                        get: |a| a.traction_control.label().to_owned(),
                        set: |a, v| a.traction_control = aid(v),
                    },
                    AIDS,
                    "Factory",
                ),
                range(
                    "Stability control",
                    Key::Assist {
                        get: |a| a.stability.to_string(),
                        set: |a, v| a.stability = amount(v).min(100),
                    },
                    (0., 100., 1.),
                    1.,
                    percent_or_off,
                    "0",
                ),
                toggle(
                    "Ideal line",
                    Key::Assist {
                        get: |a| flag(a.ideal_line),
                        set: |a, v| a.ideal_line = is_on(v),
                    },
                    false,
                )
                .note("Draws the racing line on the track, coloured by speed."),
                toggle(
                    "Auto clutch",
                    Key::Assist {
                        get: |a| flag(a.auto_clutch),
                        set: |a, v| a.auto_clutch = is_on(v),
                    },
                    false,
                ),
                toggle(
                    "Automatic gearbox",
                    Key::Assist {
                        get: |a| flag(a.auto_shifter),
                        set: |a, v| a.auto_shifter = is_on(v),
                    },
                    false,
                ),
                toggle(
                    "Auto blip",
                    Key::Assist {
                        get: |a| flag(a.auto_blip),
                        set: |a, v| a.auto_blip = is_on(v),
                    },
                    false,
                )
                .note("Matches revs for you on downshifts."),
            ],
        },
        Section {
            title: "Realism",
            right: true,
            defs: &[
                range(
                    "Damage",
                    Key::Assist {
                        get: |a| a.damage.to_string(),
                        set: |a, v| a.damage = amount(v).min(100),
                    },
                    (0., 100., 1.),
                    1.,
                    percent_or_off,
                    "100",
                ),
                choice(
                    "Tyre wear",
                    Key::Assist {
                        get: |a| a.tyre_wear.to_string(),
                        set: |a, v| a.tyre_wear = amount(v),
                    },
                    RATES,
                    "1",
                ),
                choice(
                    "Fuel use",
                    Key::Assist {
                        get: |a| a.fuel_use.to_string(),
                        set: |a, v| a.fuel_use = amount(v),
                    },
                    RATES,
                    "1",
                ),
                toggle(
                    "Tyre blankets",
                    Key::Assist {
                        get: |a| flag(a.tyre_blankets),
                        set: |a, v| a.tyre_blankets = is_on(v),
                    },
                    true,
                )
                .note("Tyres start the session warm."),
            ],
        },
    ],
};

pub const APPS_PAGE: Page = Page {
    name: "In-game apps",
    blurb: "Windows and readouts on screen while you drive.",
    files: &[PYTHON],
    extra: Extra::Apps,
    sections: &[Section {
        title: "On screen",
        right: true,
        defs: &[
            toggle(
                "Live delta",
                ini(GAMEPLAY, "TIME_DIFFERENCE", "IS_ACTIVE"),
                true,
            )
            .note("The gap to your best lap, as you drive."),
            toggle("Sector times", ini(GAMEPLAY, "GUI", "SHOW_SECTORS"), true),
            toggle(
                "Names above cars",
                ini(GAMEPLAY, "DRIVER_NAME_DISPLAYER", "IS_ACTIVE"),
                false,
            ),
            toggle(
                "Virtual mirror",
                ini(GAMEPLAY, "VIRTUAL_MIRROR", "ACTIVE"),
                false,
            ),
            toggle(
                "Pit limiter light only in the pit lane",
                ini(GAMEPLAY, "PIT_INDICATOR", "ONLY_IN_PITLINE"),
                true,
            ),
            toggle(
                "Refused downshift warning",
                ini(GAMEPLAY, "DOWNSHIFT_PROTECTION_NOTIFICATION", "ACTIVE"),
                true,
            ),
            toggle(
                "Steering centred notice",
                ini(GAMEPLAY, "STEERING_CENTERED_NOTIFICATION", "ACTIVE"),
                true,
            ),
            toggle(
                "Let app windows overlap",
                ini(GAMEPLAY, "GUI", "ALLOW_OVERLAPPING_FORMS"),
                false,
            ),
        ],
    }],
};

/// The switch above the list of Python apps.
pub const PYTHON_APPS: Def = toggle(
    "Python apps",
    ini(GAMEPLAY, "PYTHON", "ENABLE_PYTHON"),
    true,
)
.note("Off runs the game without any of the apps below.");

const SAVED: &[(&str, &str)] = &[
    ("-1", "All"),
    ("0", "None"),
    ("1", "Last 1"),
    ("2", "Last 2"),
    ("3", "Last 3"),
    ("5", "Last 5"),
    ("10", "Last 10"),
];

pub const SYSTEM_PAGE: Page = Page {
    name: "System",
    blurb: "Units, replays and the settings files themselves.",
    files: &[GAMEPLAY, REPLAY],
    extra: Extra::Files,
    sections: &[
        Section {
            title: "General",
            right: false,
            defs: &[choice(
                "Units",
                ini(GAMEPLAY, "OPTIONS", "USE_MPH"),
                &[("0", "Metric (km/h)"), ("1", "Imperial (mph)")],
                "0",
            )],
        },
        Section {
            title: "Replays",
            right: false,
            defs: &[
                choice(
                    "Recording rate",
                    ini(REPLAY, "QUALITY", "LEVEL"),
                    &[
                        ("0", "7 Hz"),
                        ("1", "10 Hz"),
                        ("2", "15 Hz"),
                        ("3", "30 Hz"),
                        ("4", "60 Hz"),
                    ],
                    "3",
                ),
                range(
                    "Memory for replays",
                    ini(REPLAY, "REPLAY", "MAX_SIZE_MB"),
                    (10., 1000., 10.),
                    1.,
                    megabytes,
                    "26",
                ),
                toggle(
                    "Save replays automatically",
                    ini(REPLAY, "AUTOSAVE", "ENABLED"),
                    true,
                ),
                choice("Races kept", ini(REPLAY, "AUTOSAVE", "RACE"), SAVED, "2"),
                choice(
                    "Qualifying kept",
                    ini(REPLAY, "AUTOSAVE", "QUALIFY"),
                    SAVED,
                    "1",
                ),
                choice(
                    "Other sessions kept",
                    ini(REPLAY, "AUTOSAVE", "OTHERS"),
                    SAVED,
                    "1",
                ),
                range(
                    "Shortest kept",
                    ini(REPLAY, "AUTOSAVE", "MIN_TIME_SECONDS"),
                    (0., 300., 5.),
                    1.,
                    seconds,
                    "30",
                ),
            ],
        },
    ],
};

// Controls, by input method.

pub const FORCE_FEEDBACK: &[Def] = &[
    range(
        "Gain",
        ini(CONTROLS, "STEER", "FF_GAIN"),
        (0., 200., 1.),
        100.,
        percent,
        "1",
    ),
    range(
        "Filter",
        ini(CONTROLS, "STEER", "FILTER_FF"),
        (0., 100., 1.),
        100.,
        percent,
        "0",
    ),
    range(
        "Minimum force",
        ini(CONTROLS, "FF_TWEAKS", "MIN_FF"),
        (0., 30., 1.),
        100.,
        percent,
        "0.05",
    ),
    range(
        "Kerb effect",
        ini(CONTROLS, "FF_ENHANCEMENT", "CURBS"),
        (0., 100., 1.),
        100.,
        percent,
        "0.4",
    ),
    range(
        "Road effect",
        ini(CONTROLS, "FF_ENHANCEMENT", "ROAD"),
        (0., 100., 1.),
        100.,
        percent,
        "0.5",
    ),
    range(
        "Slip effect",
        ini(CONTROLS, "FF_ENHANCEMENT", "SLIPS"),
        (0., 100., 1.),
        100.,
        percent,
        "0",
    ),
    range(
        "ABS effect",
        ini(CONTROLS, "FF_ENHANCEMENT", "ABS"),
        (0., 100., 1.),
        100.,
        percent,
        "0.21",
    ),
    range(
        "Skip steps",
        ini(CONTROLS, "FF_SKIP_STEPS", "VALUE"),
        (0., 5., 1.),
        1.,
        whole_or_off,
        "0",
    ),
    toggle(
        "Enhanced understeer effect",
        ini(CONTROLS, "FF_ENHANCEMENT_2", "UNDERSTEER"),
        false,
    )
    .note("The wheel goes light as the front tyres lose grip."),
];

pub const STEERING: &[Def] = &[
    range(
        "Rotation",
        ini(CONTROLS, "STEER", "LOCK"),
        (180., 1080., 10.),
        1.,
        degrees,
        "900",
    ),
    range(
        "Gamma",
        ini(CONTROLS, "STEER", "STEER_GAMMA"),
        (1., 3., 0.1),
        1.,
        tenths,
        "1",
    ),
    range(
        "Filter",
        ini(CONTROLS, "STEER", "STEER_FILTER"),
        (0., 100., 1.),
        100.,
        percent,
        "0",
    ),
    range(
        "Speed sensitivity",
        ini(CONTROLS, "STEER", "SPEED_SENSITIVITY"),
        (0., 100., 1.),
        100.,
        percent,
        "0",
    ),
    range(
        "Brake gamma",
        ini(CONTROLS, "BRAKES", "GAMMA"),
        (1., 5., 0.1),
        1.,
        tenths,
        "2.4",
    ),
    toggle(
        "Keyboard as well",
        ini(CONTROLS, "ADVANCED", "COMBINE_WITH_KEYBOARD_CONTROL"),
        false,
    )
    .note("Keys keep working alongside the wheel."),
];

pub const GAMEPAD: &[Def] = &[
    choice(
        "Steer with",
        ini(CONTROLS, "X360", "STEER_THUMB"),
        &[("LEFT", "Left stick"), ("RIGHT", "Right stick")],
        "LEFT",
    ),
    range(
        "Steering gamma",
        ini(CONTROLS, "X360", "STEER_GAMMA"),
        (1., 5., 0.1),
        1.,
        tenths,
        "2",
    ),
    range(
        "Steering filter",
        ini(CONTROLS, "X360", "STEER_FILTER"),
        (0., 100., 1.),
        100.,
        percent,
        "0.6",
    ),
    range(
        "Speed sensitivity",
        ini(CONTROLS, "X360", "SPEED_SENSITIVITY"),
        (0., 100., 1.),
        100.,
        percent,
        "0.5",
    ),
    range(
        "Dead zone",
        ini(CONTROLS, "X360", "STEER_DEADZONE"),
        (0., 50., 1.),
        100.,
        percent,
        "0.05",
    ),
    range(
        "Rumble",
        ini(CONTROLS, "X360", "RUMBLE_INTENSITY"),
        (0., 100., 1.),
        100.,
        percent_or_off,
        "1",
    ),
];

pub const KEYBOARD: &[Def] = &[
    range(
        "Steering speed",
        ini(CONTROLS, "KEYBOARD", "STEERING_SPEED"),
        (0.5, 5., 0.05),
        1.,
        hundredths,
        "1.75",
    ),
    range(
        "Counter-steer speed",
        ini(CONTROLS, "KEYBOARD", "STEERING_OPPOSITE_DIRECTION_SPEED"),
        (0.5, 5., 0.05),
        1.,
        hundredths,
        "2.5",
    ),
    range(
        "Return speed",
        ini(CONTROLS, "KEYBOARD", "STEER_RESET_SPEED"),
        (0.5, 5., 0.05),
        1.,
        hundredths,
        "1.8",
    ),
    range(
        "Mouse speed",
        ini(CONTROLS, "KEYBOARD", "MOUSE_SPEED"),
        (0.01, 1., 0.01),
        1.,
        hundredths,
        "0.1",
    ),
    toggle(
        "Steer with the mouse",
        ini(CONTROLS, "KEYBOARD", "MOUSE_STEER"),
        false,
    ),
    toggle(
        "Mouse buttons for throttle and brake",
        ini(CONTROLS, "KEYBOARD", "MOUSE_ACCELERATOR_BRAKE"),
        false,
    ),
];

fn faces(value: f32) -> String {
    match value as u32 {
        0 => "Static".to_owned(),
        6 => "Every frame".to_owned(),
        n => format!("{n} of 6 per frame"),
    }
}

const RESOLUTIONS: [(u32, u32); 13] = [
    (1280, 720),
    (1366, 768),
    (1600, 900),
    (1920, 1080),
    (1920, 1200),
    (2560, 1080),
    (2560, 1440),
    (2560, 1600),
    (3440, 1440),
    (3840, 2160),
    (5120, 1440),
    (5760, 1080),
    (7680, 1440),
];

fn resolutions(_: &Game) -> Vec<(String, String)> {
    RESOLUTIONS
        .iter()
        .map(|(w, h)| (format!("{w}x{h}"), format!("{w} × {h}")))
        .collect()
}

fn filters(game: &Game) -> Vec<(String, String)> {
    Config::pp_filters(&game.install)
        .into_iter()
        .map(|name| (name.clone(), name.replace('_', " ")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assist_settings_round_trip() {
        let mut assists = muretto_core::session::AssistPreset::Gamer.assists();
        for def in ASSISTS_PAGE.sections.iter().flat_map(|s| s.defs) {
            let Key::Assist { get, set } = def.key else {
                panic!("{} is not an assist", def.label);
            };
            let before = assists.clone();
            set(&mut assists, &get(&before));
            assert_eq!(assists, before, "{}", def.label);
        }
    }
}
