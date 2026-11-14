//! Settings › Assetto Corsa › Controls: the input method, how the wheel's
//! axes and buttons are bound, force feedback and steering, and presets.

use gpui_kit::{
    AnyElement, Context, Div, Entity, FontWeight, IntoElement, PathPromptOptions, Render,
    SharedString, Subscription, TestSupportExt as _, Window,
    component::input::{Input, InputState},
    div,
    prelude::*,
    px, rgb,
};
use muretto_core::settings::{
    self, FF_POST,
    controls::{self, ACTIONS, Axis, InputMethod, Preset},
};

use super::{
    form::{Form, switch_row},
    pages::{FORCE_FEEDBACK, GAMEPAD, KEYBOARD, STEERING},
};
use crate::{
    game::Game,
    theme::{GAP, PAGE, RADIUS, color, font},
    ui::{ChoiceField, button, display, label, mono, muted, panel},
};

pub struct ControlsPage {
    game: Entity<Game>,
    force_feedback: Form,
    steering: Form,
    gamepad: Form,
    keyboard: Form,
    presets: Vec<Preset>,
    preset: ChoiceField<String>,
    /// The name box, while a preset is being saved.
    naming: Option<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
}

impl ControlsPage {
    pub fn new(game: Entity<Game>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let presets = controls::presets(&game.read(cx).install);
        let preset = ChoiceField::new(
            preset_options(&presets),
            &String::new(),
            window,
            cx,
            |this: &mut Self, path, _, cx| this.load(path, cx),
        );
        let _subscriptions = vec![cx.observe_in(&game, window, |this, game, window, cx| {
            for form in [
                &this.force_feedback,
                &this.steering,
                &this.gamepad,
                &this.keyboard,
            ] {
                form.sync(&game, window, cx);
            }
            cx.notify();
        })];
        Self {
            force_feedback: Form::new(FORCE_FEEDBACK, &game, window, cx),
            steering: Form::new(STEERING, &game, window, cx),
            gamepad: Form::new(GAMEPAD, &game, window, cx),
            keyboard: Form::new(KEYBOARD, &game, window, cx),
            presets,
            preset,
            naming: None,
            game,
            _subscriptions,
        }
    }

    pub fn terms(&self) -> Vec<SharedString> {
        let mut terms: Vec<SharedString> = [
            "Wheel",
            "Gamepad",
            "Keyboard",
            "Force feedback",
            "Preset",
            "Axes",
            "Buttons",
            "Invert",
            "LUT",
        ]
        .into_iter()
        .map(SharedString::from)
        .collect();
        terms.extend(Axis::ALL.map(|axis| SharedString::from(axis.label())));
        terms.extend(
            ACTIONS
                .iter()
                .map(|action| SharedString::from(action.label)),
        );
        for form in [
            &self.force_feedback,
            &self.steering,
            &self.gamepad,
            &self.keyboard,
        ] {
            terms.extend(form.labels().map(SharedString::from));
        }
        terms
    }

    fn change(
        &self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut settings::Config, &muretto_core::Install) -> muretto_core::Result<()>,
    ) {
        self.game.update(cx, |game, cx| {
            let result = game.configure(cx, change);
            game.config_error = result.err().map(|e| e.to_string().into());
        });
    }

    fn load(&mut self, path: String, cx: &mut Context<Self>) {
        if path.is_empty() {
            return;
        }
        self.change(cx, move |config, _| {
            controls::load_preset(config, std::path::Path::new(&path))
        });
    }

    fn start_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Preset name"));
        input.update(cx, |input, cx| input.focus(window, cx));
        self.naming = Some(input);
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.naming.take() else {
            return;
        };
        let name = input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.naming = Some(input);
            return;
        }
        let mut saved = None;
        self.change(cx, |config, install| {
            saved = Some(controls::save_preset(config, install, &name)?);
            Ok(())
        });
        self.presets = controls::presets(&self.game.read(cx).install);
        let selected = saved
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.preset
            .set_options(preset_options(&self.presets), &selected, window, cx);
        cx.notify();
    }

    fn import_curve(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use curve".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.change(cx, move |config, _| config.import_ff_curve(&path).map(drop));
            })
            .ok();
        })
        .detach();
    }

    fn header(&self, method: InputMethod, cx: &mut Context<Self>) -> Div {
        let ini = self.game.read(cx).config.file(settings::CONTROLS).clone();
        let devices = controls::devices(&ini);
        let preset_actions = match &self.naming {
            Some(input) => div()
                .flex()
                .gap(px(8.))
                .child(div().w(px(200.)).child(Styled::h(
                    Input::new(input).id("preset-name").bg(rgb(color::BG)),
                    px(32.),
                )))
                .child(
                    button("preset-save", "Save")
                        .light()
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                )
                .child(button("preset-cancel", "Cancel").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.naming = None;
                        cx.notify();
                    },
                ))),
            None => div().child(
                button("preset-save-as", "Save as…")
                    .on_click(cx.listener(|this, _, window, cx| this.start_naming(window, cx))),
            ),
        };

        div()
            .flex()
            .items_start()
            .justify_between()
            .gap(GAP)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(display("Controls", 27.))
                    .child(if devices.is_empty() || method != InputMethod::Wheel {
                        muted("Bindings are made in the game's Controls screen; set everything else here.")
                    } else {
                        div().flex().flex_wrap().gap(px(16.)).children(devices.into_iter().map(
                            |(_, name)| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(div().size(px(7.)).rounded_full().bg(rgb(color::TEXT_DIM)))
                                    .child(muted(name))
                            },
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .pt(px(8.))
                            .children(InputMethod::ALL.into_iter().map(|choice| {
                                let id = SharedString::from(format!(
                                    "input-{}",
                                    choice.label().to_lowercase()
                                ));
                                let chosen = choice == method;
                                let tab = button(id, choice.label()).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.change(cx, move |config, _| {
                                            config.set(
                                                settings::CONTROLS,
                                                "HEADER",
                                                "INPUT_METHOD",
                                                choice.value(),
                                            )
                                        });
                                    },
                                ));
                                if chosen { tab.light() } else { tab }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("Preset"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().w(px(240.)).child(self.preset.element_or("Load a preset…")))
                            .child(preset_actions),
                    ),
            )
    }

    fn axes_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let ini = self.game.read(cx).config.file(settings::CONTROLS).clone();
        let rows = Axis::ALL.into_iter().enumerate().map(|(index, axis)| {
            let binding = controls::axis(&ini, axis);
            let bound = binding.bound_to.is_some();
            let mut bound_to = binding.bound_to.unwrap_or_else(|| "Not bound".to_owned());
            if let Some(gamma) = binding.gamma.filter(|_| bound) {
                bound_to = format!("{bound_to} · gamma {gamma}");
            }
            div()
                .flex()
                .items_center()
                .gap(GAP)
                .h(px(48.))
                .px(px(16.))
                .border_t_1()
                .border_color(rgb(color::BORDER))
                .child(
                    div()
                        .w(px(110.))
                        .flex_none()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(axis.label()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .text_color(rgb(if bound {
                            color::TEXT_MUTED
                        } else {
                            color::TEXT_DIM
                        }))
                        .truncate()
                        .child(bound_to),
                )
                .child(
                    div().w(px(140.)).flex_none().child(
                        switch_row(("axis-invert", index), "Invert", None, binding.inverted, {
                            let game = self.game.clone();
                            move |on, _, cx| {
                                game.update(cx, |game, cx| {
                                    let result = game.configure(cx, |config, _| {
                                        controls::set_inverted(config, axis, on)
                                    });
                                    game.config_error = result.err().map(|e| e.to_string().into());
                                });
                            }
                        })
                        .when(!bound, |row| row.opacity(0.5)),
                    ),
                )
        });
        panel("Axes")
            .child(
                div()
                    .flex()
                    .gap(GAP)
                    .h(px(34.))
                    .items_center()
                    .px(px(16.))
                    .child(div().w(px(110.)).flex_none().child(label("Axis")))
                    .child(div().flex_1().child(label("Bound to")))
                    .child(div().w(px(140.)).flex_none()),
            )
            .children(rows)
            .into_any_element()
    }

    fn buttons_panel(&self, method: InputMethod, cx: &mut Context<Self>) -> AnyElement {
        let ini = self.game.read(cx).config.file(settings::CONTROLS).clone();
        let mut bound: Vec<(&str, String)> = Vec::new();
        if method == InputMethod::Keyboard {
            for (name, key) in [
                ("Throttle", "GAS"),
                ("Brake", "BRAKE"),
                ("Steer left", "LEFT"),
                ("Steer right", "RIGHT"),
            ] {
                if let Some(binding) = ini.get("KEYBOARD", key).and_then(controls::key_name) {
                    bound.push((name, binding));
                }
            }
        }
        bound.extend(ACTIONS.iter().filter_map(|action| {
            controls::binding(&ini, action, method).map(|binding| (action.label, binding))
        }));
        let unbound = ACTIONS.len()
            + if method == InputMethod::Keyboard {
                4
            } else {
                0
            }
            - bound.len();

        let mut grid = div().flex().flex_col();
        for (row, pair) in bound.chunks(2).enumerate() {
            grid = grid.child(
                div()
                    .flex()
                    .gap(GAP)
                    .px(px(16.))
                    .py(px(7.))
                    .when(row > 0, |line| {
                        line.border_t_1().border_color(rgb(color::BORDER))
                    })
                    .children(pair.iter().map(|(name, binding)| {
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .child(div().text_size(px(13.)).truncate().child(*name))
                            .child(
                                div()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .border_1()
                                    .border_color(rgb(color::BORDER_STRONG))
                                    .rounded(RADIUS)
                                    .child(mono(binding.clone(), 12.)),
                            )
                    }))
                    .when(pair.len() == 1, |line| line.child(div().flex_1())),
            );
        }
        panel("Buttons")
            .when(bound.is_empty(), |panel| {
                panel.child(div().p(px(16.)).child(muted("Nothing is bound yet.")))
            })
            .child(grid)
            .when(unbound > 0, |panel| {
                panel.child(
                    div()
                        .px(px(16.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(rgb(color::BORDER))
                        .child(muted(format!(
                            "{unbound} more actions are not bound. Bind buttons in the game's own Controls screen."
                        ))),
                )
            })
            .into_any_element()
    }

    fn force_feedback_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let config = &self.game.read(cx).config;
        let curve = (config.flag(FF_POST, "HEADER", "ENABLED") == Some(true)
            && config.get(FF_POST, "HEADER", "TYPE").as_deref() == Some("LUT"))
        .then(|| config.get(FF_POST, "LUT", "CURVE"))
        .flatten();
        let curve_row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(GAP)
            .px(px(16.))
            .pb(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .child("Post-processing curve (LUT)"),
                    )
                    .child(muted(match &curve {
                        Some(name) => format!("Using {name}"),
                        None => "None".to_owned(),
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .when(curve.is_some(), |row| {
                        row.child(button("ff-curve-off", "Turn off").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.change(cx, |config, _| {
                                    config.set(FF_POST, "HEADER", "ENABLED", "0")
                                })
                            },
                        )))
                    })
                    .child(
                        button("ff-curve-import", "Import…")
                            .on_click(cx.listener(|this, _, _, cx| this.import_curve(cx))),
                    ),
            );
        panel("Force feedback")
            .child(self.force_feedback.body(&self.game, cx))
            .child(curve_row)
            .into_any_element()
    }
}

impl Render for ControlsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let method = InputMethod::of(self.game.read(cx).config.file(settings::CONTROLS));
        let (left, right): (Vec<AnyElement>, Vec<AnyElement>) = match method {
            InputMethod::Wheel => (
                vec![self.axes_panel(cx), self.buttons_panel(method, cx)],
                vec![
                    self.force_feedback_panel(cx),
                    panel("Steering")
                        .child(self.steering.body(&self.game, cx))
                        .into_any_element(),
                ],
            ),
            InputMethod::Gamepad => (
                vec![self.buttons_panel(method, cx)],
                vec![
                    panel("Gamepad")
                        .child(self.gamepad.body(&self.game, cx))
                        .into_any_element(),
                ],
            ),
            InputMethod::Keyboard => (
                vec![self.buttons_panel(method, cx)],
                vec![
                    panel("Keyboard and mouse")
                        .child(self.keyboard.body(&self.game, cx))
                        .into_any_element(),
                ],
            ),
        };
        let header = self.header(method, cx);

        div()
            .id("page-controls")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP)
                    .p(PAGE)
                    .font_family(font::BODY)
                    .child(header)
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(GAP)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(GAP)
                                    .children(left),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .w(px(464.))
                                    .flex_none()
                                    .gap(GAP)
                                    .children(right),
                            ),
                    ),
            )
            .test_support()
    }
}

fn preset_options(presets: &[Preset]) -> Vec<(String, SharedString)> {
    presets
        .iter()
        .map(|preset| {
            let name = if preset.own {
                format!("{} · yours", preset.name)
            } else {
                preset.name.clone()
            };
            (preset.path.display().to_string(), SharedString::from(name))
        })
        .collect()
}
