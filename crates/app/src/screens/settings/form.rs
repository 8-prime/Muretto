//! Settings described as data: each [`Def`] names where a value lives and
//! how it is edited, and a [`Form`] turns a list of them into controls that
//! read from and write to the game's files.

use gpui_kit::{
    AnyElement, App, Context, Div, Entity, IntoElement, SharedString, Window, div, prelude::*, px,
    relative, rgb,
};
use muretto_core::{session::Assists, settings};

use crate::{
    game::Game,
    theme::{GAP, color},
    ui::{ChoiceField, RangeField, label, mono, switch},
};

/// Where a value lives.
pub enum Key {
    Ini {
        file: &'static str,
        section: &'static str,
        key: &'static str,
    },
    /// `WIDTH` and `HEIGHT` in `video.ini`, as `1920x1080`.
    Resolution,
    /// `FPS_CAP_MS` in `video.ini`, as frames per second; `0` for none.
    FpsLimit,
    /// One of the session's assists, also kept in `assists.ini`.
    Assist {
        get: fn(&Assists) -> String,
        set: fn(&mut Assists, &str),
    },
}

pub const fn ini(file: &'static str, section: &'static str, key: &'static str) -> Key {
    Key::Ini { file, section, key }
}

pub enum Kind {
    Toggle,
    /// A slider showing the stored number times `scale`.
    Range {
        min: f32,
        max: f32,
        step: f32,
        scale: f32,
        show: fn(f32) -> String,
    },
    Choice(Options),
}

pub enum Options {
    /// Stored value and what the player sees.
    Fixed(&'static [(&'static str, &'static str)]),
    /// Options that depend on what is installed.
    Found(fn(&Game) -> Vec<(String, String)>),
}

pub struct Def {
    pub label: &'static str,
    pub key: Key,
    pub kind: Kind,
    /// What the game assumes when the file does not say.
    pub default: &'static str,
    /// A line under a switch, explaining it.
    pub note: Option<&'static str>,
}

impl Def {
    pub const fn note(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }

    /// A stable element id, for tests and the screenshot tool.
    pub fn id(&self) -> SharedString {
        let slug: String = self
            .label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        format!("setting-{slug}").into()
    }
}

pub const fn toggle(label: &'static str, key: Key, default: bool) -> Def {
    Def {
        label,
        key,
        kind: Kind::Toggle,
        default: if default { "1" } else { "0" },
        note: None,
    }
}

pub const fn range(
    label: &'static str,
    key: Key,
    (min, max, step): (f32, f32, f32),
    scale: f32,
    show: fn(f32) -> String,
    default: &'static str,
) -> Def {
    Def {
        label,
        key,
        kind: Kind::Range {
            min,
            max,
            step,
            scale,
            show,
        },
        default,
        note: None,
    }
}

pub const fn choice(
    label: &'static str,
    key: Key,
    options: &'static [(&'static str, &'static str)],
    default: &'static str,
) -> Def {
    Def {
        label,
        key,
        kind: Kind::Choice(Options::Fixed(options)),
        default,
        note: None,
    }
}

pub const fn found(
    label: &'static str,
    key: Key,
    options: fn(&Game) -> Vec<(String, String)>,
    default: &'static str,
) -> Def {
    Def {
        label,
        key,
        kind: Kind::Choice(Options::Found(options)),
        default,
        note: None,
    }
}

// How ranges show their value.

pub fn percent(value: f32) -> String {
    format!("{value:.0}%")
}

pub fn percent_or_off(value: f32) -> String {
    if value <= 0. {
        "Off".to_owned()
    } else {
        percent(value)
    }
}

pub fn whole(value: f32) -> String {
    format!("{value:.0}")
}

pub fn whole_or_off(value: f32) -> String {
    if value <= 0. {
        "Off".to_owned()
    } else {
        whole(value)
    }
}

pub fn tenths(value: f32) -> String {
    format!("{value:.1}")
}

pub fn hundredths(value: f32) -> String {
    format!("{value:.2}")
}

pub fn degrees(value: f32) -> String {
    format!("{value:.0}°")
}

pub fn metres(value: f32) -> String {
    format!("{value:.0} m")
}

pub fn megabytes(value: f32) -> String {
    format!("{value:.0} MB")
}

pub fn seconds(value: f32) -> String {
    format!("{value:.0} s")
}

/// The value as the game currently has it.
pub fn read(def: &Def, game: &Game) -> String {
    let config = &game.config;
    let value = match def.key {
        Key::Ini { file, section, key } => config.get(file, section, key),
        Key::Resolution => {
            let width = config.number(settings::VIDEO, "VIDEO", "WIDTH");
            let height = config.number(settings::VIDEO, "VIDEO", "HEIGHT");
            width.zip(height).map(|(w, h)| format!("{w:.0}x{h:.0}"))
        }
        Key::FpsLimit => config
            .number(settings::VIDEO, "VIDEO", "FPS_CAP_MS")
            .map(|ms| settings::fps_from_cap(ms).unwrap_or(0).to_string()),
        Key::Assist { get, .. } => Some(get(&game.session().assists)),
    };
    value
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| def.default.to_owned())
}

/// Writes a new value straight to the game's files.
pub fn write(def: &'static Def, game: &Entity<Game>, value: String, cx: &mut App) {
    game.update(cx, |game, cx| {
        let result = match def.key {
            Key::Ini { file, section, key } => {
                game.configure(cx, |config, _| config.set(file, section, key, &value))
            }
            Key::Resolution => {
                let (width, height) = value.split_once('x').unwrap_or(("1920", "1080"));
                game.configure(cx, |config, _| {
                    config.edit(settings::VIDEO, |ini| {
                        ini.set("VIDEO", "WIDTH", width)
                            .set("VIDEO", "HEIGHT", height);
                    })
                })
            }
            Key::FpsLimit => {
                let cap = settings::cap_from_fps(value.parse().ok());
                game.configure(cx, |config, _| {
                    config.set(settings::VIDEO, "VIDEO", "FPS_CAP_MS", &cap)
                })
            }
            Key::Assist { set, .. } => game.set_assists(cx, |assists| set(assists, &value)),
        };
        game.config_error = result.err().map(|e| e.to_string().into());
    });
}

fn is_on(value: &str) -> bool {
    value
        .trim()
        .parse::<f64>()
        .map_or(value == "true", |n| n != 0.)
}

/// The listed option `value` stands for: the same text, or the same number
/// written differently (`1` and `1.0`).
fn matching<'a>(options: &'a [(String, String)], value: &str) -> Option<&'a str> {
    let number = value.trim().parse::<f64>().ok();
    options
        .iter()
        .find(|(v, _)| {
            v.eq_ignore_ascii_case(value)
                || number
                    .zip(v.parse::<f64>().ok())
                    .is_some_and(|(a, b)| (a - b).abs() <= 1e-3 * a.abs().max(1.))
        })
        .map(|(v, _)| v.as_str())
}

fn options(def: &Def, game: &Game) -> Vec<(String, String)> {
    let Kind::Choice(options) = &def.kind else {
        return Vec::new();
    };
    let mut list = match options {
        Options::Fixed(list) => list
            .iter()
            .map(|(v, l)| ((*v).to_owned(), (*l).to_owned()))
            .collect(),
        Options::Found(find) => find(game),
    };
    // Keep a value set some other way choosable rather than showing nothing.
    let current = read(def, game);
    if matching(&list, &current).is_none() {
        list.push((current.clone(), current));
    }
    list
}

enum Control {
    Toggle,
    Range(RangeField),
    Choice(ChoiceField<String>),
}

/// Controls for a list of settings, laid out as one panel's body.
pub struct Form {
    defs: Vec<&'static Def>,
    controls: Vec<Control>,
}

impl Form {
    pub fn new<V: 'static>(
        defs: &'static [Def],
        game: &Entity<Game>,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let controls = defs
            .iter()
            .map(|def| {
                let value = read(def, game.read(cx));
                let game = game.clone();
                match def.kind {
                    Kind::Toggle => Control::Toggle,
                    Kind::Range {
                        min,
                        max,
                        step,
                        scale,
                        ..
                    } => Control::Range(RangeField::new(
                        (min, max, step),
                        value.parse::<f32>().unwrap_or(min / scale) * scale,
                        cx,
                        move |_, shown, cx| write(def, &game, number_text(shown / scale), cx),
                    )),
                    Kind::Choice(_) => {
                        let list = options(def, game.read(cx));
                        let selected = matching(&list, &value).unwrap_or_default().to_owned();
                        Control::Choice(ChoiceField::new(
                            list.into_iter()
                                .map(|(value, label)| (value, SharedString::from(label)))
                                .collect(),
                            &selected,
                            window,
                            cx,
                            move |_, value, _, cx| write(def, &game, value, cx),
                        ))
                    }
                }
            })
            .collect();
        Self {
            defs: defs.iter().collect(),
            controls,
        }
    }

    pub fn labels(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.defs.iter().map(|def| def.label)
    }

    /// Brings the controls in line with the files, after they changed
    /// some other way.
    pub fn sync(&self, game: &Entity<Game>, window: &mut Window, cx: &mut App) {
        let state = game.read(cx);
        let values: Vec<Option<String>> = self
            .defs
            .iter()
            .map(|def| {
                let value = read(def, state);
                match def.kind {
                    Kind::Choice(_) => matching(&options(def, state), &value).map(str::to_owned),
                    _ => Some(value),
                }
            })
            .collect();
        for ((def, control), value) in self.defs.iter().zip(&self.controls).zip(values) {
            let Some(value) = value else { continue };
            match (control, &def.kind) {
                (Control::Range(field), Kind::Range { scale, .. }) => {
                    if let Ok(stored) = value.parse::<f32>() {
                        field.set(stored * scale, window, cx);
                    }
                }
                (Control::Choice(field), _) => field.select(&value, window, cx),
                _ => {}
            }
        }
    }

    /// Sliders and lists two to a row, each under its label; switches one
    /// to a row with the label beside them.
    pub fn body(&self, game: &Entity<Game>, cx: &App) -> Div {
        let state = game.read(cx);
        let mut body = div().flex().flex_col().gap(px(14.)).p(px(16.));
        let mut pending: Vec<AnyElement> = Vec::new();
        let flush = |body: Div, pending: &mut Vec<AnyElement>| {
            if pending.is_empty() {
                return body;
            }
            let mut cells = std::mem::take(pending);
            while cells.len() < 2 {
                cells.push(div().flex_1().into_any_element());
            }
            body.child(div().flex().gap(GAP).children(cells))
        };
        for (&def, control) in self.defs.iter().zip(&self.controls) {
            let value = read(def, state);
            match (control, &def.kind) {
                (Control::Toggle, _) => {
                    body = flush(body, &mut pending);
                    let game = game.clone();
                    body = body.child(toggle_row(def, is_on(&value), move |on, _, cx| {
                        write(def, &game, if on { "1" } else { "0" }.to_owned(), cx)
                    }));
                }
                (Control::Range(field), Kind::Range { scale, show, .. }) => {
                    let shown = value
                        .parse::<f32>()
                        .map_or(String::new(), |v| show(v * scale));
                    pending.push(cell(def.label, Some(shown), field.element(def.label, cx)));
                }
                (Control::Choice(field), _) => {
                    pending.push(cell(def.label, None, field.element()));
                }
                _ => {}
            }
            if pending.len() == 2 {
                body = flush(body, &mut pending);
            }
        }
        flush(body, &mut pending)
    }
}

/// A label, its value at the right, and the control below.
fn cell(name: &str, value: Option<String>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(label(name))
                .when_some(value, |row, value| row.child(mono(value, 12.))),
        )
        .child(control)
        .into_any_element()
}

/// A setting that is on or off, with its switch at the right.
pub fn toggle_row(
    def: &Def,
    on: bool,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Div {
    switch_row(def.id(), def.label, def.note, on, on_toggle)
}

pub fn switch_row(
    id: impl Into<gpui_kit::ElementId>,
    title: impl Into<SharedString>,
    note: Option<&'static str>,
    on: bool,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(GAP)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(13.5)).child(title.into()))
                .when_some(note, |text, note| {
                    text.child(
                        div()
                            .text_size(px(12.))
                            .line_height(relative(1.4))
                            .text_color(rgb(color::TEXT_MUTED))
                            .child(note),
                    )
                }),
        )
        .child(switch(id, on).on_toggle(on_toggle))
}

/// A number as the game writes it: no trailing zeros.
pub fn number_text(value: f32) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(number_text(0.78), "0.78");
        assert_eq!(number_text(5.), "5");
        assert_eq!(number_text(-0.), "0");
        assert_eq!(number_text(1.0 / 3.0), "0.3333");
    }

    #[test]
    fn options_match_numbers_written_differently() {
        let list = vec![
            ("1".to_owned(), "On".to_owned()),
            ("16.6667".to_owned(), "60".to_owned()),
        ];
        assert_eq!(matching(&list, "1.0"), Some("1"));
        assert_eq!(matching(&list, "16.666666"), Some("16.6667"));
        assert_eq!(matching(&list, "2"), None);
    }
}
