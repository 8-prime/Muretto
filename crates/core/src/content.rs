//! Cars, tracks and weather as installed on disk.

use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::{Install, ini::Ini, lenient};

/// Who made a piece of content. Shown as a badge next to its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Kunos,
    Dlc,
    Mod,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Kunos => "Kunos",
            Origin::Dlc => "DLC",
            Origin::Mod => "Mod",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Car {
    /// Folder name; what the game calls the car.
    pub id: String,
    pub dir: PathBuf,
    pub name: String,
    pub brand: String,
    /// `race` or `street`, as written in the file.
    pub class: String,
    pub tags: Vec<String>,
    pub country: Option<String>,
    pub year: Option<u16>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub description: String,
    pub specs: CarSpecs,
    /// (rpm, bhp)
    pub power_curve: Vec<(f32, f32)>,
    /// (rpm, Nm)
    pub torque_curve: Vec<(f32, f32)>,
    pub skins: Vec<Skin>,
    pub origin: Origin,
    /// False for DLC the player has not bought: the folder holds only the
    /// showroom data.
    pub owned: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CarSpecs {
    pub power_bhp: Option<f64>,
    pub torque_nm: Option<f64>,
    pub weight_kg: Option<f64>,
    pub top_speed_kmh: Option<f64>,
    pub acceleration_s: Option<f64>,
    pub weight_per_bhp: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub id: String,
    pub name: String,
    pub number: Option<String>,
    pub team: Option<String>,
    pub driver: Option<String>,
    /// Showroom render. Not every skin ships one.
    pub preview: Option<PathBuf>,
    /// Small square swatch. Not every skin ships one.
    pub livery: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: String,
    pub dir: PathBuf,
    /// Name shared by all layouts, e.g. "Brands Hatch".
    pub name: String,
    /// At least one. Single-layout tracks have one layout with no id.
    pub layouts: Vec<TrackLayout>,
    pub origin: Origin,
    pub owned: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackLayout {
    /// The `CONFIG_TRACK` value; `None` for single-layout tracks.
    pub id: Option<String>,
    /// Full name as written in the file, e.g. "Brands Hatch - GP".
    pub full_name: String,
    /// What distinguishes this layout from its siblings, e.g. "GP".
    pub name: String,
    pub description: String,
    pub country: Option<String>,
    pub city: Option<String>,
    pub length_m: Option<f64>,
    pub pit_boxes: Option<u32>,
    /// `clockwise` / `anticlockwise`, as written in the file.
    pub direction: Option<String>,
    pub tags: Vec<String>,
    pub preview: Option<PathBuf>,
    pub outline: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Weather {
    pub id: String,
    pub name: String,
    /// How strongly this weather heats (positive) or cools (negative) the
    /// road relative to the air; see [`road_temperature`].
    pub temperature_coefficient: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    pub cars: Vec<Car>,
    pub tracks: Vec<Track>,
    pub weather: Vec<Weather>,
}

impl Library {
    /// Reads every car, track and weather in the install. Entries that cannot
    /// be read are skipped rather than failing the scan: one broken mod must
    /// not hide the rest of the collection.
    pub fn scan(install: &Install) -> Self {
        let mut cars: Vec<Car> = subdirs(&install.cars_dir())
            .iter()
            .filter_map(|dir| Car::load(dir))
            .collect();
        cars.sort_by_cached_key(|car| car.name.to_lowercase());

        let mut tracks: Vec<Track> = subdirs(&install.tracks_dir())
            .iter()
            .filter_map(|dir| Track::load(dir))
            .collect();
        tracks.sort_by_cached_key(|track| track.name.to_lowercase());

        let weather = subdirs(&install.weather_dir())
            .iter()
            .filter_map(|dir| Weather::load(dir))
            .collect();

        Library {
            cars,
            tracks,
            weather,
        }
    }

    pub fn car(&self, id: &str) -> Option<&Car> {
        self.cars.iter().find(|car| car.id == id)
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn weather(&self, id: &str) -> Option<&Weather> {
        self.weather.iter().find(|weather| weather.id == id)
    }

    /// Cars that can share a grid with `car`: those with the same racing
    /// class, or just the car itself when it has none.
    pub fn same_class<'a>(&'a self, car: &Car) -> Vec<&'a Car> {
        let class = car.class_label();
        self.cars
            .iter()
            .filter(|other| other.owned && other.class_label() == class && other.class == car.class)
            .collect()
    }
}

impl Car {
    pub fn load(dir: &Path) -> Option<Self> {
        let id = dir.file_name()?.to_str()?.to_owned();
        let ui = lenient::read_json(&dir.join("ui/ui_car.json")).ok()?;
        let tags = lenient::text_list(&ui, "tags");
        let specs = ui.get("specs").cloned().unwrap_or(Value::Null);
        let is_dlc = dir.join("ui/dlc_ui_car.json").is_file();
        let author = lenient::text(&ui, "author");
        let origin = if is_dlc {
            Origin::Dlc
        } else if is_kunos_car(&id)
            || author
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case("kunos"))
        {
            Origin::Kunos
        } else {
            Origin::Mod
        };

        Some(Car {
            name: lenient::text(&ui, "name").unwrap_or_else(|| id.clone()),
            brand: lenient::text(&ui, "brand").unwrap_or_default(),
            class: lenient::text(&ui, "class")
                .unwrap_or_default()
                .to_lowercase(),
            country: lenient::text(&ui, "country").or_else(|| country_from_tags(&tags)),
            year: lenient::number(&ui, "year")
                .map(|y| y as u16)
                .filter(|y| (1880..2100).contains(y)),
            author,
            version: lenient::text(&ui, "version"),
            description: lenient::text(&ui, "description").unwrap_or_default(),
            specs: CarSpecs {
                power_bhp: lenient::number(&specs, "bhp"),
                torque_nm: lenient::number(&specs, "torque"),
                weight_kg: lenient::number(&specs, "weight"),
                top_speed_kmh: lenient::number(&specs, "topspeed"),
                acceleration_s: lenient::number(&specs, "acceleration"),
                weight_per_bhp: lenient::number(&specs, "pwratio"),
            },
            power_curve: curve(&ui, "powerCurve"),
            torque_curve: curve(&ui, "torqueCurve"),
            skins: subdirs(&dir.join("skins"))
                .iter()
                .filter_map(|dir| Skin::load(dir))
                .collect(),
            owned: dir.join("data.acd").is_file() || dir.join("data").is_dir(),
            origin,
            tags,
            dir: dir.to_path_buf(),
            id,
        })
    }

    /// The racing class if the tags name one ("GT2", "DTM"), otherwise the
    /// broad category ("Street", "Race").
    pub fn class_label(&self) -> String {
        const CLASSES: &[(&str, &str)] = &[
            ("gt2", "GT2"),
            ("gte", "GTE"),
            ("gt3", "GT3"),
            ("gt4", "GT4"),
            ("gt1", "GT1"),
            ("lmp1", "LMP1"),
            ("lmp2", "LMP2"),
            ("lmp3", "LMP3"),
            ("dtm", "DTM"),
            ("f1", "F1"),
            ("formula", "Formula"),
            ("group c", "Group C"),
            ("groupc", "Group C"),
            ("group a", "Group A"),
            ("rally", "Rally"),
            ("drift", "Drift"),
            ("cup", "Cup"),
            ("touring", "Touring"),
            ("prototype", "Prototype"),
            ("hypercar", "Hypercar"),
            ("supercar", "Supercar"),
        ];
        let tags: Vec<String> = self.tags.iter().map(|t| t.to_lowercase()).collect();
        CLASSES
            .iter()
            .find(|(tag, _)| tags.iter().any(|t| t == tag))
            .map(|(_, label)| (*label).to_owned())
            .unwrap_or_else(|| capitalize(&self.class))
    }

    /// The description as plain text: the line breaks and entities the
    /// game's own files carry become spaces and characters.
    pub fn plain_description(&self) -> String {
        plain_text(&self.description)
    }

    /// "RWD", "AWD" or "FWD" when the tags say so.
    pub fn drivetrain(&self) -> Option<&'static str> {
        self.tags
            .iter()
            .find_map(|tag| match tag.to_lowercase().as_str() {
                "rwd" => Some("RWD"),
                "awd" | "4wd" => Some("AWD"),
                "fwd" => Some("FWD"),
                _ => None,
            })
    }

    pub fn skin(&self, id: &str) -> Option<&Skin> {
        self.skins.iter().find(|skin| skin.id == id)
    }
}

impl Skin {
    fn load(dir: &Path) -> Option<Self> {
        let id = dir.file_name()?.to_str()?.to_owned();
        let ui = lenient::read_json(&dir.join("ui_skin.json")).unwrap_or(Value::Null);
        Some(Skin {
            name: lenient::text(&ui, "skinname").unwrap_or_else(|| name_from_id(&id)),
            number: lenient::text(&ui, "number").filter(|n| n != "0"),
            team: lenient::text(&ui, "team"),
            driver: lenient::text(&ui, "drivername"),
            preview: existing(dir.join("preview.jpg")),
            livery: existing(dir.join("livery.png")),
            id,
        })
    }

    /// "Rosso #62", or just the name when the skin has no number.
    pub fn label(&self) -> String {
        match &self.number {
            Some(number) => format!("{} #{number}", self.name),
            None => self.name.clone(),
        }
    }
}

impl Track {
    pub fn load(dir: &Path) -> Option<Self> {
        let id = dir.file_name()?.to_str()?.to_owned();
        let ui = dir.join("ui");
        let mut layouts = Vec::new();
        let mut is_dlc = false;
        if ui.join("ui_track.json").is_file() {
            is_dlc |= ui.join("dlc_ui_track.json").is_file();
            layouts.extend(TrackLayout::load(&ui, None));
        }
        for layout_dir in subdirs(&ui) {
            is_dlc |= layout_dir.join("dlc_ui_track.json").is_file();
            let layout_id = layout_dir.file_name()?.to_str()?.to_owned();
            layouts.extend(TrackLayout::load(&layout_dir, Some(layout_id)));
        }
        if layouts.is_empty() {
            return None;
        }

        let name = shared_name(&layouts);
        for layout in &mut layouts {
            let own = layout
                .full_name
                .strip_prefix(name.as_str())
                .unwrap_or(&layout.full_name)
                .trim_start_matches([' ', '-', '–', '—', ':'])
                .trim();
            layout.name = if own.is_empty() {
                layout.full_name.clone()
            } else {
                own.to_owned()
            };
        }

        let owned = std::fs::read_dir(dir).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|ext| ext == "kn5"))
        });
        let origin = if is_dlc {
            Origin::Dlc
        } else if is_kunos_track(&id) {
            Origin::Kunos
        } else {
            Origin::Mod
        };
        Some(Track {
            id,
            dir: dir.to_path_buf(),
            name,
            layouts,
            origin,
            owned,
        })
    }

    /// The layout with this `CONFIG_TRACK` id, or the first one.
    pub fn layout(&self, id: Option<&str>) -> &TrackLayout {
        self.layouts
            .iter()
            .find(|layout| layout.id.as_deref() == id)
            .unwrap_or(&self.layouts[0])
    }
}

impl TrackLayout {
    fn load(dir: &Path, id: Option<String>) -> Option<Self> {
        let ui = lenient::read_json(&dir.join("ui_track.json")).ok()?;
        let full_name = lenient::text(&ui, "name")?;
        Some(TrackLayout {
            name: full_name.clone(),
            full_name,
            description: lenient::text(&ui, "description").unwrap_or_default(),
            country: lenient::text(&ui, "country"),
            city: lenient::text(&ui, "city"),
            length_m: ui.get("length").and_then(length_in_metres),
            pit_boxes: lenient::number(&ui, "pitboxes").map(|n| n as u32),
            direction: lenient::text(&ui, "run").map(|run| run.to_lowercase()),
            tags: lenient::text_list(&ui, "tags"),
            preview: existing(dir.join("preview.png")),
            outline: existing(dir.join("outline.png")),
            id,
        })
    }
}

impl Weather {
    fn load(dir: &Path) -> Option<Self> {
        let id = dir.file_name()?.to_str()?.to_owned();
        let bytes = std::fs::read(dir.join("weather.ini")).ok()?;
        let ini = Ini::parse(&lenient::decode(&bytes));
        Some(Weather {
            name: ini
                .get("LAUNCHER", "NAME")
                .map(sentence_case)
                .unwrap_or_else(|| name_from_id(&id)),
            temperature_coefficient: ini
                .get_number("LAUNCHER", "TEMPERATURE_COEFF")
                .unwrap_or(0.0),
            id,
        })
    }
}

/// Road surface temperature in °C, by the game's own formula
/// (`content/weather/readme_weather.txt`): the road runs warmer than the air
/// under a clear sky and cooler under cloud, most strongly around midday.
pub fn road_temperature(
    ambient_c: f64,
    seconds_since_midnight: u32,
    weather_coefficient: f64,
) -> f64 {
    let hours = f64::from(seconds_since_midnight) / 3600.0;
    let x = ((hours - 7.0) * 0.04167).clamp(0.0, 1.0);
    let lift = 5.33332
        * weather_coefficient
        * (1.0 - x)
        * ((-6.0 * x).exp() * (6.0 * x).sin() + 0.25)
        * (0.9 * x).sin();
    ambient_c * (1.0 + lift)
}

/// Lengths are metres in the stock files ("7004") but mods write "5.793 km",
/// "5793m" and bare kilometre figures.
fn length_in_metres(value: &Value) -> Option<f64> {
    let (number, text) = match value {
        Value::Number(n) => (n.as_f64()?, String::new()),
        Value::String(s) => (lenient::leading_number(s)?, s.to_lowercase()),
        _ => return None,
    };
    if number <= 0.0 {
        return None;
    }
    let is_km = text.contains("km") || number < 100.0;
    Some(if is_km { number * 1000.0 } else { number })
}

fn curve(ui: &Value, key: &str) -> Vec<(f32, f32)> {
    let as_number = |v: &Value| match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    };
    ui.get(key)
        .and_then(Value::as_array)
        .map(|points| {
            points
                .iter()
                .filter_map(|point| {
                    let point = point.as_array()?;
                    Some((
                        as_number(point.first()?)? as f32,
                        as_number(point.get(1)?)? as f32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The longest prefix all layout names share, cut at a word boundary:
/// "Brands Hatch - GP" and "Brands Hatch - Indy" share "Brands Hatch".
fn shared_name(layouts: &[TrackLayout]) -> String {
    let first = &layouts[0].full_name;
    if layouts.len() == 1 {
        return first.clone();
    }
    let mut len = first.len();
    for layout in &layouts[1..] {
        len = first
            .char_indices()
            .zip(layout.full_name.chars())
            .find(|((_, a), b)| a != b)
            .map(|((i, _), _)| i)
            .unwrap_or(first.len().min(layout.full_name.len()))
            .min(len);
    }
    let mut prefix = &first[..len];
    // Do not split a word: "Sprint A" / "Sprint B" share "Sprint ", but
    // "Highlands" / "Highlands Long" share the whole first name.
    let separators = [' ', '-', '–', '—', ':'];
    let ends_cleanly = prefix.ends_with(separators)
        || layouts.iter().all(|l| {
            l.full_name[len..]
                .chars()
                .next()
                .is_none_or(|c| separators.contains(&c))
        });
    if !ends_cleanly {
        prefix = prefix.rfind(' ').map_or("", |i| &prefix[..i]);
    }
    // "Nurburgring - Sprint A" / "Nurburgring - Sprint B": the track is what
    // precedes the separator, not "Nurburgring - Sprint".
    if let Some(separator) = prefix.find(" - ") {
        prefix = &prefix[..separator];
    }
    let prefix = prefix.trim_end_matches([' ', '-', '–', '—', ':']);
    if prefix.is_empty() {
        first.clone()
    } else {
        prefix.to_owned()
    }
}

fn country_from_tags(tags: &[String]) -> Option<String> {
    const COUNTRIES: &[&str] = &[
        "italy",
        "germany",
        "japan",
        "france",
        "usa",
        "great britain",
        "uk",
        "sweden",
        "austria",
        "spain",
        "netherlands",
        "belgium",
        "korea",
        "australia",
        "czech republic",
        "china",
    ];
    tags.iter().find_map(|tag| {
        let tag = tag.to_lowercase();
        COUNTRIES
            .contains(&tag.as_str())
            .then(|| match tag.as_str() {
                "usa" => "USA".to_owned(),
                "uk" => "Great Britain".to_owned(),
                other => other
                    .split(' ')
                    .map(capitalize)
                    .collect::<Vec<_>>()
                    .join(" "),
            })
    })
}

/// "00_rosso_corsa" -> "Rosso corsa"
fn name_from_id(id: &str) -> String {
    let stripped = id.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_');
    let words = if stripped.is_empty() { id } else { stripped };
    sentence_case(&words.replace(['_', '-'], " "))
}

fn sentence_case(text: &str) -> String {
    capitalize(&text.to_lowercase())
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn is_kunos_car(id: &str) -> bool {
    id.starts_with("ks_") || KUNOS_CARS.binary_search(&id).is_ok()
}

fn is_kunos_track(id: &str) -> bool {
    id.starts_with("ks_") || KUNOS_TRACKS.contains(&id)
}

/// Stock cars that predate the `ks_` naming scheme. Sorted.
const KUNOS_CARS: &[&str] = &[
    "abarth500",
    "abarth500_s1",
    "alfa_romeo_giulietta_qv",
    "alfa_romeo_giulietta_qv_le",
    "bmw_1m",
    "bmw_1m_s3",
    "bmw_m3_e30",
    "bmw_m3_e30_drift",
    "bmw_m3_e30_dtm",
    "bmw_m3_e30_gra",
    "bmw_m3_e30_s1",
    "bmw_m3_e92",
    "bmw_m3_e92_drift",
    "bmw_m3_e92_s1",
    "bmw_m3_gt2",
    "bmw_z4",
    "bmw_z4_drift",
    "bmw_z4_gt3",
    "bmw_z4_s1",
    "ferrari_312t",
    "ferrari_458",
    "ferrari_458_gt2",
    "ferrari_458_s3",
    "ferrari_599xxevo",
    "ferrari_f40",
    "ferrari_f40_s3",
    "ferrari_laferrari",
    "ktm_xbow_r",
    "lotus_2_eleven",
    "lotus_2_eleven_gt4",
    "lotus_49",
    "lotus_98t",
    "lotus_elise_sc",
    "lotus_elise_sc_s1",
    "lotus_elise_sc_s2",
    "lotus_evora_gtc",
    "lotus_evora_gte",
    "lotus_evora_gte_carbon",
    "lotus_evora_gx",
    "lotus_evora_s",
    "lotus_evora_s_s2",
    "lotus_exige_240",
    "lotus_exige_240_s3",
    "lotus_exige_s",
    "lotus_exige_s_roadster",
    "lotus_exige_scura",
    "lotus_exige_v6_cup",
    "lotus_exos_125",
    "lotus_exos_125_s1",
    "mclaren_mp412c",
    "mclaren_mp412c_gt3",
    "mercedes_sls",
    "mercedes_sls_gt3",
    "p4-5_2011",
    "pagani_huayra",
    "pagani_zonda_r",
    "ruf_yellowbird",
    "shelby_cobra_427sc",
    "tatuusfa1",
];

const KUNOS_TRACKS: &[&str] = &[
    "drift",
    "imola",
    "magione",
    "monza",
    "mugello",
    "spa",
    "trento-bondone",
];

/// Markup in `ui_*.json` texts made plain: tags become line breaks or go,
/// entities become characters, and runs of blank space collapse.
pub fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(['<', '&']) {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else { break };
            let tag = rest[1..end].trim().to_ascii_lowercase();
            if matches!(tag.trim_end_matches('/').trim(), "br" | "p" | "/p" | "/br") {
                out.push('\n');
            }
            rest = &rest[end + 1..];
        } else {
            let entity = rest
                .find(';')
                .filter(|&end| end <= 8)
                .map(|end| &rest[..=end]);
            let replacement = match entity {
                Some("&quot;") => "\"",
                Some("&amp;") => "&",
                Some("&apos;" | "&#39;") => "'",
                Some("&lt;") => "<",
                Some("&gt;") => ">",
                Some("&nbsp;") => " ",
                Some("&ldquo;" | "&rdquo;") => "\"",
                Some("&lsquo;" | "&rsquo;") => "'",
                _ => {
                    out.push('&');
                    rest = &rest[1..];
                    continue;
                }
            };
            out.push_str(replacement);
            rest = &rest[entity.map_or(1, str::len)..];
        }
    }
    out.push_str(rest);
    // One blank line at most between paragraphs, no stray spaces.
    let mut paragraphs: Vec<String> = Vec::new();
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() {
            paragraphs.push(line);
        }
    }
    paragraphs.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_lose_their_markup() {
        assert_eq!(
            plain_text(
                "The &quot;458&quot;  is fast.<br><br/></br>Tom &amp; Jerry & co <b>bold</b>"
            ),
            "The \"458\" is fast.\nTom & Jerry & co bold"
        );
        assert_eq!(plain_text("a < b"), "a < b");
    }

    fn layout(name: &str) -> TrackLayout {
        TrackLayout {
            id: None,
            full_name: name.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            country: None,
            city: None,
            length_m: None,
            pit_boxes: None,
            direction: None,
            tags: Vec::new(),
            preview: None,
            outline: None,
        }
    }

    #[test]
    fn kunos_car_list_is_sorted_for_binary_search() {
        assert!(KUNOS_CARS.is_sorted());
        assert!(is_kunos_car("ferrari_458_gt2"));
        assert!(is_kunos_car("ks_porsche_911_gt3_r_2016"));
        assert!(!is_kunos_car("kw_s2000_ap1_track"));
    }

    #[test]
    fn layouts_share_the_track_name() {
        assert_eq!(
            shared_name(&[layout("Brands Hatch - GP"), layout("Brands Hatch - Indy")]),
            "Brands Hatch"
        );
        assert_eq!(
            shared_name(&[
                layout("Nurburgring - Sprint A"),
                layout("Nurburgring - Sprint B")
            ]),
            "Nurburgring"
        );
        assert_eq!(shared_name(&[layout("Spa")]), "Spa");
        assert_eq!(
            shared_name(&[
                layout("Highlands Drift"),
                layout("Highlands"),
                layout("Highlands Long")
            ]),
            "Highlands"
        );
        // Nothing in common: keep the first name rather than an empty title.
        assert_eq!(shared_name(&[layout("Full"), layout("Junior")]), "Full");
    }

    #[test]
    fn track_lengths_are_normalised_to_metres() {
        assert_eq!(length_in_metres(&Value::from("7004")), Some(7004.0));
        assert_eq!(length_in_metres(&Value::from("5.793 km")), Some(5793.0));
        assert_eq!(length_in_metres(&Value::from("5793m")), Some(5793.0));
        assert_eq!(length_in_metres(&Value::from(20.8)), Some(20800.0));
        assert_eq!(length_in_metres(&Value::from("")), None);
    }

    #[test]
    fn skin_names_fall_back_to_the_folder() {
        assert_eq!(name_from_id("00_red"), "Red");
        assert_eq!(name_from_id("rosso_corsa"), "Rosso corsa");
        assert_eq!(
            name_from_id("2011_Team_Sofrev_ASP_20"),
            "Team sofrev asp 20"
        );
        assert_eq!(name_from_id("0"), "0");
    }

    #[test]
    fn road_runs_warmer_than_air_under_clear_sky_and_cooler_under_cloud() {
        let afternoon = 14 * 3600 + 30 * 60;
        assert_eq!(road_temperature(22.0, afternoon, 1.0).round(), 31.0);
        assert_eq!(road_temperature(22.0, afternoon, -0.3).round(), 19.0);
        assert_eq!(road_temperature(22.0, afternoon, 0.0), 22.0);
    }
}
