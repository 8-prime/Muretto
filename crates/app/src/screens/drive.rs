//! Drive › Quick drive: pick a car, a track and the conditions, then go.

use std::path::Path;

use gpui_kit::{
    App, Context, Div, Entity, EventEmitter, IntoElement, ObjectFit, Render, SharedString,
    TestSupportExt as _, Window, div, img, pattern_slash, prelude::*, px, rgb,
};
use muretto_core::{
    Library, SessionConfig,
    session::{
        AssistPreset, GridKind, SessionMode, TIME_MAX, TIME_MIN, TrackSurface, compass, format_time,
    },
};

use crate::{
    game::Game,
    theme::{GAP, PAGE, RADIUS, color, font, hsla},
    ui::{
        ChoiceField, NumberField, RangeField, badge, button, choice_tile, display, label, mono,
        muted, panel, stat_strip, switch,
    },
};

/// Requests the screen cannot fulfil itself because they cover the window.
pub enum DriveEvent {
    ChooseCar,
    ChooseTrack,
}

pub struct DriveScreen {
    game: Entity<Game>,

    laps: NumberField,
    opponents: NumberField,
    start_position: NumberField,
    layout: ChoiceField<Option<String>>,
    grid: ChoiceField<GridKind>,
    weather: ChoiceField<String>,
    surface: ChoiceField<TrackSurface>,
    assists: ChoiceField<AssistPreset>,
    ai_strength: RangeField,
    ai_aggression: RangeField,
    time: RangeField,
    ambient: RangeField,
    wind: RangeField,

    /// What the layout and grid lists were last built for.
    listed_track: String,
    listed_car: String,
    all_skins: bool,
}

impl EventEmitter<DriveEvent> for DriveScreen {}

/// Skins shown before the rest fold into a "+N" tile.
const SKINS_SHOWN: usize = 8;

impl DriveScreen {
    pub fn new(game: Entity<Game>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = game.read(cx);
        let session = state.session().clone();
        let library = state.library.clone();
        let layouts = layout_options(state);
        let grids = grid_options(&session, &library);

        cx.observe_in(&game, window, |this, _, window, cx| {
            this.sync(window, cx);
            cx.notify();
        })
        .detach();

        Self {
            laps: NumberField::new(session.laps, window, cx, |this, laps, _, cx| {
                this.edit(cx, |s| s.laps = laps)
            }),
            opponents: NumberField::new(session.opponents, window, cx, |this, n, _, cx| {
                this.edit(cx, |s| s.opponents = n)
            }),
            start_position: NumberField::new(
                session.start_position,
                window,
                cx,
                |this, n, _, cx| this.edit(cx, |s| s.start_position = n),
            ),
            layout: ChoiceField::new(
                layouts,
                &session.layout_id,
                window,
                cx,
                |this, id, _, cx| this.edit(cx, |s| s.layout_id = id),
            ),
            grid: ChoiceField::new(grids, &session.grid, window, cx, |this, grid, _, cx| {
                this.edit(cx, |s| s.grid = grid)
            }),
            weather: ChoiceField::new(
                library
                    .weather
                    .iter()
                    .map(|w| (w.id.clone(), SharedString::from(w.name.clone())))
                    .collect(),
                &session.weather_id,
                window,
                cx,
                |this, id, _, cx| this.edit(cx, |s| s.weather_id = id),
            ),
            surface: ChoiceField::new(
                TrackSurface::ALL
                    .into_iter()
                    .map(|s| (s, SharedString::from(s.label())))
                    .collect(),
                &session.surface,
                window,
                cx,
                |this, surface, _, cx| this.edit(cx, |s| s.surface = surface),
            ),
            assists: ChoiceField::new(
                AssistPreset::ALL
                    .into_iter()
                    .map(|p| (p, SharedString::from(p.label())))
                    .collect(),
                &session.assists.preset().unwrap_or(AssistPreset::Pro),
                window,
                cx,
                |this, preset, _, cx| this.edit(cx, |s| s.assists = preset.assists()),
            ),
            ai_strength: RangeField::new(
                (70., 100., 1.),
                session.ai_strength as f32,
                cx,
                |this, v, cx| this.edit(cx, |s| s.ai_strength = v as u32),
            ),
            ai_aggression: RangeField::new(
                (0., 100., 1.),
                session.ai_aggression as f32,
                cx,
                |this, v, cx| this.edit(cx, |s| s.ai_aggression = v as u32),
            ),
            // Quarter-hour steps across the hours the game can light.
            time: RangeField::new(
                (TIME_MIN as f32, TIME_MAX as f32, 900.),
                session.time_seconds as f32,
                cx,
                |this, v, cx| this.edit(cx, |s| s.time_seconds = v as u32),
            ),
            ambient: RangeField::new(
                (0., 40., 1.),
                session.ambient_c as f32,
                cx,
                |this, v, cx| this.edit(cx, |s| s.ambient_c = v as i32),
            ),
            wind: RangeField::new((0., 40., 1.), session.wind_kmh as f32, cx, |this, v, cx| {
                this.edit(cx, |s| s.wind_kmh = v as u32)
            }),
            listed_track: session.track_id.clone(),
            listed_car: session.car_id.clone(),
            all_skins: false,
            game,
        }
    }

    /// Brings the controls back in line after the session changed elsewhere:
    /// a new car or track, or a value clamped into range.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let game = self.game.read(cx);
        let session = game.session().clone();
        let library = game.library.clone();
        let layouts = (self.listed_track != session.track_id).then(|| layout_options(game));

        if let Some(layouts) = layouts {
            self.layout
                .set_options(layouts, &session.layout_id, window, cx);
            self.listed_track = session.track_id.clone();
        }
        if self.listed_car != session.car_id {
            self.grid
                .set_options(grid_options(&session, &library), &session.grid, window, cx);
            self.listed_car = session.car_id.clone();
            self.all_skins = false;
        }
        self.laps.set(session.laps, window, cx);
        self.opponents.set(session.opponents, window, cx);
        self.start_position.set(session.start_position, window, cx);
    }

    fn edit(&self, cx: &mut App, change: impl FnOnce(&mut SessionConfig)) {
        self.game.update(cx, |game, cx| game.edit(cx, change));
    }

    fn car_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let game = self.game.read(cx);
        let car = game.car();
        let session = game.session();
        let skin = game.skin();
        let specs = &car.specs;
        let figure = |value: Option<f64>, unit: &str| -> SharedString {
            value.map_or("—".into(), |v| format!("{v:.0} {unit}").into())
        };
        let meta = [
            Some(car.brand.clone()),
            car.country.clone(),
            car.year.map(|y| y.to_string()),
            Some(car.class_label()),
        ];

        let shown = if self.all_skins {
            car.skins.len()
        } else {
            SKINS_SHOWN
        };
        let hidden = car.skins.len().saturating_sub(shown);
        let swatches = car
            .skins
            .iter()
            .take(shown)
            .enumerate()
            .map(|(index, skin)| {
                let id = skin.id.clone();
                let chosen = skin.id == session.skin_id;
                div()
                    .id(("skin", index))
                    .flex_none()
                    .w(px(36.))
                    .h(px(28.))
                    .rounded(RADIUS)
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(if chosen { color::TEXT } else { color::BORDER }))
                    .bg(rgb(color::BG))
                    .cursor_pointer()
                    .when_some(skin.livery.clone(), |tile, livery| {
                        tile.child(img(livery).size_full().object_fit(ObjectFit::Cover))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = id.clone();
                        this.edit(cx, |s| s.skin_id = id);
                    }))
            });

        panel("Car")
            .flex_1()
            .action(
                button("change-car", "Change car").on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(DriveEvent::ChooseCar);
                })),
            )
            .child(preview(skin.and_then(|s| s.preview.as_deref()), None))
            .child(
                body()
                    .child(title_row(&car.name, car.origin.label()))
                    .child(muted(join(meta)).mt(px(-10.)))
                    .child(stat_strip([
                        ("Power", figure(specs.power_bhp, "bhp")),
                        ("Torque", figure(specs.torque_nm, "Nm")),
                        ("Weight", figure(specs.weight_kg, "kg")),
                        ("Drive", car.drivetrain().unwrap_or("—").into()),
                    ]))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(label(&match skin {
                                Some(skin) => format!("Livery · {}", skin.label()),
                                None => "Livery".to_owned(),
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(6.))
                                    .children(swatches)
                                    .when(hidden > 0, |row| {
                                        row.child(
                                            div()
                                                .id("more-skins")
                                                .flex()
                                                .flex_none()
                                                .items_center()
                                                .justify_center()
                                                .w(px(40.))
                                                .h(px(28.))
                                                .rounded(RADIUS)
                                                .border_1()
                                                .border_color(rgb(color::BORDER))
                                                .cursor_pointer()
                                                .hover(|style| style.bg(rgb(color::RAISED)))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.all_skins = true;
                                                    cx.notify();
                                                }))
                                                .child(
                                                    mono(format!("+{hidden}"), 12.)
                                                        .text_color(rgb(color::TEXT_MUTED)),
                                                ),
                                        )
                                    }),
                            ),
                    ),
            )
    }

    fn track_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let game = self.game.read(cx);
        let track = game.track();
        let layout = game.layout();
        let kind = layout
            .tags
            .iter()
            .map(|tag| tag.to_lowercase())
            .find(|tag| {
                [
                    "circuit",
                    "street",
                    "hillclimb",
                    "drift",
                    "drag",
                    "rally",
                    "oval",
                ]
                .contains(&tag.as_str())
            })
            .map(|tag| capitalize(&tag));
        let meta = [
            layout.country.clone(),
            kind,
            layout.direction.as_deref().map(capitalize),
        ];

        panel("Track")
            .flex_1()
            .action(
                button("change-track", "Change track").on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(DriveEvent::ChooseTrack);
                })),
            )
            .child(preview(
                layout.preview.as_deref(),
                layout.outline.as_deref(),
            ))
            .child(
                body()
                    .child(title_row(&track.name, track.origin.label()))
                    .child(muted(join(meta)).mt(px(-10.)))
                    .child(stat_strip([
                        (
                            "Length",
                            layout
                                .length_m
                                .map_or("—".into(), |m| format!("{:.3} km", m / 1000.).into()),
                        ),
                        (
                            "Pit boxes",
                            layout
                                .pit_boxes
                                .map_or("—".into(), |n| n.to_string().into()),
                        ),
                        ("Layouts", track.layouts.len().to_string().into()),
                    ]))
                    .child(field("Layout", self.layout.element())),
            )
    }

    fn session_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.game.read(cx).session().clone();
        let mode = session.mode;
        let tiles = |modes: &[SessionMode]| {
            div().flex().gap(px(6.)).children(modes.iter().map(|&m| {
                choice_tile(("mode", m as usize), m.label(), m == mode)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.game.update(cx, |game, cx| game.set_mode(m, cx));
                    }))
                    .test_support()
            }))
        };
        let game = self.game.clone();
        let toggle = |id: &'static str,
                      on: bool,
                      caption: &'static str,
                      set: fn(&mut SessionConfig, bool)| {
            let game = game.clone();
            switch(id, on).caption(caption).on_toggle(move |on, _, cx| {
                game.update(cx, |game, cx| game.edit(cx, |s| set(s, on)));
            })
        };

        panel("Session").w(px(493.)).flex_none().child(
            body()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(tiles(&SessionMode::ALL[..4]))
                        .child(tiles(&SessionMode::ALL[4..])),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(field("Laps", self.laps.element("laps", mode.has_laps())))
                        .child(field(
                            "Opponents",
                            self.opponents.element("opponents", mode.has_opponents()),
                        ))
                        .child(field(
                            "Start position",
                            self.start_position
                                .element("start-position", mode.has_grid()),
                        )),
                )
                .child(
                    field("Grid", self.grid.element())
                        .when(!mode.has_opponents(), |this| this.opacity(0.45)),
                )
                .child(
                    div()
                        .flex()
                        .gap(GAP)
                        .when(!mode.has_opponents(), |this| this.opacity(0.45))
                        .child(slider_field(
                            "AI strength",
                            format!("{}%", session.ai_strength),
                            self.ai_strength.element("AI strength", cx),
                        ))
                        .child(slider_field(
                            "AI aggression",
                            format!("{}%", session.ai_aggression),
                            self.ai_aggression.element("AI aggression", cx),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(24.))
                        .child(toggle(
                            "penalties",
                            session.penalties,
                            "Penalties",
                            |s, on| s.penalties = on,
                        ))
                        .child(toggle(
                            "shuffle-ai",
                            session.shuffle_ai_skill,
                            "Shuffle AI skill",
                            |s, on| s.shuffle_ai_skill = on,
                        )),
                ),
        )
    }

    fn conditions_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let game = self.game.read(cx);
        let session = game.session();
        let road = session.road_c(&game.library);
        let column = || div().flex().flex_col().flex_1().min_w_0().gap(px(6.));

        panel("Conditions").flex_1().child(
            body()
                .child(
                    div()
                        .flex()
                        .gap(GAP)
                        .child(
                            column()
                                .child(slider_field(
                                    "Time of day",
                                    format_time(session.time_seconds),
                                    self.time.element("Time of day", cx),
                                ))
                                .child(caption(format!("Time flow ×{}", session.time_multiplier))),
                        )
                        .child(
                            column()
                                .child(field("Weather", self.weather.element()))
                                .child(caption("Fixed for the whole session")),
                        )
                        .child(
                            column()
                                .child(slider_field(
                                    "Ambient",
                                    format!("{} °C", session.ambient_c),
                                    self.ambient.element("Ambient temperature", cx),
                                ))
                                .child(caption(format!("Road surface {road} °C"))),
                        )
                        .child(
                            column()
                                .child(slider_field(
                                    "Wind",
                                    format!("{} km/h", session.wind_kmh),
                                    self.wind.element("Wind speed", cx),
                                ))
                                .child(caption(format!(
                                    "From {}, {}°",
                                    compass(session.wind_direction_deg),
                                    session.wind_direction_deg
                                ))),
                        ),
                )
                .child(
                    div().flex().gap(GAP).child(
                        column()
                            .flex_none()
                            .w(px(200.))
                            .child(field("Track surface", self.surface.element()))
                            .child(caption(session.surface.summary())),
                    ),
                ),
        )
    }

    fn assists_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let assists = self.game.read(cx).session().assists.clone();
        let on_off = |on: bool| if on { "On" } else { "Off" }.to_owned();
        let percent = |value: u32| {
            if value == 0 {
                "Off".to_owned()
            } else {
                format!("{value}%")
            }
        };
        let rate = |value: u32| {
            if value == 0 {
                "Off".to_owned()
            } else {
                format!("×{value}")
            }
        };
        let row = |cells: [(&str, String); 3]| {
            div().flex().gap(GAP).children(cells.map(|(name, value)| {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(label(name))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(color::TEXT))
                            .child(value),
                    )
            }))
        };

        panel("Assists & realism")
            .w(px(493.))
            .flex_none()
            .action(div().w(px(110.)).child(self.assists.element()))
            .child(
                body()
                    .child(row([
                        ("ABS", assists.abs.label().to_owned()),
                        (
                            "Traction control",
                            assists.traction_control.label().to_owned(),
                        ),
                        ("Stability", percent(assists.stability)),
                    ]))
                    .child(row([
                        ("Damage", percent(assists.damage)),
                        ("Tyre wear", rate(assists.tyre_wear)),
                        ("Fuel use", rate(assists.fuel_use)),
                    ]))
                    .child(row([
                        ("Ideal line", on_off(assists.ideal_line)),
                        ("Auto clutch", on_off(assists.auto_clutch)),
                        ("Tyre blankets", on_off(assists.tyre_blankets)),
                    ])),
            )
    }
}

impl Render for DriveScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("drive").size_full().overflow_y_scroll().child(
            div()
                .flex()
                .flex_col()
                .gap(GAP)
                .p(PAGE)
                .child(
                    div()
                        .flex()
                        .gap(GAP)
                        .child(self.car_panel(cx))
                        .child(self.track_panel(cx))
                        .child(self.session_panel(cx)),
                )
                .child(
                    div()
                        .flex()
                        .gap(GAP)
                        .child(self.conditions_panel(cx))
                        .child(self.assists_panel(cx)),
                ),
        )
    }
}

fn layout_options(game: &Game) -> Vec<(Option<String>, SharedString)> {
    game.track()
        .layouts
        .iter()
        .map(|layout| (layout.id.clone(), SharedString::from(layout.name.clone())))
        .collect()
}

fn grid_options(session: &SessionConfig, library: &Library) -> Vec<(GridKind, SharedString)> {
    let Some(car) = library.car(&session.car_id) else {
        return Vec::new();
    };
    let class = library.same_class(car).len();
    vec![
        (
            GridKind::SameClass,
            format!(
                "Same class — {} ({class} {})",
                car.class_label(),
                if class == 1 { "car" } else { "cars" }
            )
            .into(),
        ),
        (GridKind::SameCar, format!("Same car — {}", car.name).into()),
    ]
}

/// The picture strip at the top of the car and track panels: the image
/// cropped to fill, with the track outline laid over its right half.
fn preview(image: Option<&Path>, outline: Option<&Path>) -> Div {
    div()
        .relative()
        .flex_none()
        .w_full()
        .h(px(147.))
        .overflow_hidden()
        .border_b_1()
        .border_color(rgb(color::BORDER))
        .bg(pattern_slash(hsla(color::RAISED), 10., 10.))
        .when_some(image, |strip, image| {
            strip.child(
                img(image.to_path_buf())
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
        })
        .when_some(outline, |strip, outline| {
            strip.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .h_full()
                    .w(px(190.))
                    .p(px(14.))
                    .child(
                        img(outline.to_path_buf())
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    ),
            )
        })
}

/// The padded column under a panel's title or picture.
fn body() -> Div {
    div().flex().flex_col().gap(px(18.)).p(px(16.))
}

fn title_row(name: &str, origin: &str) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .child(
            display(name, 27.)
                .flex_shrink(1.)
                .min_w_0()
                .overflow_hidden(),
        )
        .child(badge(origin).mt(px(6.)))
}

/// A control under its label.
fn field(name: &str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(8.))
        .child(label(name))
        .child(control)
}

/// A slider under its label, with the current value at the right.
fn slider_field(name: &str, value: String, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(label(name))
                .child(
                    crate::ui::tracked(&value.to_uppercase(), px(11.), 0.06)
                        .font_family(font::MONO)
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(rgb(color::TEXT)),
                ),
        )
        .child(control)
}

fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(color::TEXT_MUTED))
        .child(text.into())
}

fn join(parts: impl IntoIterator<Item = Option<String>>) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
