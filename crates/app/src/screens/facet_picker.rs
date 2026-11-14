//! A searchable list of every value one facet of the server list takes,
//! such as every track servers run, for ticking the ones to keep. Each value
//! shows how many servers it would leave, so the player chooses from what is
//! there instead of guessing what to type.

use gpui_kit::{
    Context, Entity, EventEmitter, FontWeight, IntoElement, KeyDownEvent, MouseButton, Render,
    SharedString, Subscription, TestSupportExt as _, Window,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::*,
    px, rgb, svg, uniform_list,
};
use muretto_core::online::{Facet, FacetValue};

use crate::{
    theme::{RADIUS, alpha, color, font},
    ui::{badge, button, display, switch},
};

pub enum FacetEvent {
    /// The value with this id was ticked or unticked.
    Toggled(Facet, String),
    /// Every value was unticked.
    Cleared(Facet),
    Dismissed,
}

pub struct FacetPicker {
    facet: Facet,
    values: Vec<FacetValue>,
    /// Lower-cased name and id of each value, for the search.
    haystacks: Vec<String>,
    /// Indices into `values` that pass the search and switch, in order.
    shown: Vec<usize>,
    installed_only: bool,
    search: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<FacetEvent> for FacetPicker {}

const ROW: f32 = 44.;

impl FacetPicker {
    pub fn new(
        facet: Facet,
        values: Vec<FacetValue>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(format!(
                "Search {} {}",
                values.len(),
                plural_noun(facet)
            ))
        });
        let _subscription = cx.subscribe_in(&search, window, |this, _, event, _, cx| match event {
            InputEvent::Change => {
                this.refilter(cx);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                // Enter ticks the first match, so typing a name and
                // pressing it is enough.
                if let Some(&first) = this.shown.first() {
                    this.toggle(first, cx);
                }
            }
            _ => {}
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        let haystacks = values
            .iter()
            .map(|value| format!("{} {}", value.name, value.id).to_lowercase())
            .collect();
        let mut picker = Self {
            facet,
            shown: Vec::new(),
            haystacks,
            values,
            installed_only: false,
            search,
            _subscription,
        };
        picker.refilter(cx);
        picker
    }

    fn refilter(&mut self, cx: &Context<Self>) {
        let query = self.search.read(cx).value().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        self.shown = (0..self.values.len())
            .filter(|&index| {
                let value = &self.values[index];
                // Ticked values stay in view, so they can be unticked.
                value.chosen
                    || ((!self.installed_only || value.installed == Some(true))
                        && words
                            .iter()
                            .all(|word| self.haystacks[index].contains(word)))
            })
            .collect();
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        let value = &mut self.values[index];
        value.chosen = !value.chosen;
        cx.emit(FacetEvent::Toggled(self.facet, value.id.clone()));
        cx.notify();
    }

    fn has_content(&self) -> bool {
        matches!(self.facet, Facet::Track | Facet::Car)
    }
}

fn plural_noun(facet: Facet) -> &'static str {
    match facet {
        Facet::Track => "tracks",
        Facet::Car => "cars",
        Facet::Country => "countries",
        Facet::Session => "sessions",
    }
}

impl Render for FacetPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.shown.len();
        let chosen = self.values.iter().filter(|value| value.chosen).count();
        let rows = uniform_list(
            "facet-values",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|row| {
                        let index = this.shown[row];
                        let value = &this.values[index];
                        let empty = value.count == 0;
                        div()
                            .id(("facet-value", row))
                            .flex()
                            .w_full()
                            .items_center()
                            .gap(px(12.))
                            .h(px(ROW))
                            .px(px(16.))
                            .border_b_1()
                            .border_color(rgb(color::BORDER))
                            .cursor_pointer()
                            .when(value.chosen, |row| row.bg(rgb(color::ACCENT_WASH)))
                            .hover(|style| style.bg(rgb(color::RAISED)))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(index, cx)))
                            .child(tick(value.chosen))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.))
                                    .text_color(rgb(if empty {
                                        color::TEXT_DIM
                                    } else {
                                        color::TEXT
                                    }))
                                    .child(value.name.clone()),
                            )
                            .when(value.installed == Some(true), |row| {
                                row.child(badge("Installed"))
                            })
                            .child(
                                div()
                                    .w(px(48.))
                                    .flex()
                                    .justify_end()
                                    .font_family(font::MONO)
                                    .text_size(px(12.))
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .child(value.count.to_string()),
                            )
                            .test_support()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h(px(ROW * 10.));

        let facet = self.facet;
        let installed_only = self.installed_only;
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(56.))
            .bg(alpha(0x000000, 0.6))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(FacetEvent::Dismissed)),
            )
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(FacetEvent::Dismissed);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(560.))
                    .bg(rgb(color::SURFACE))
                    .border_1()
                    .border_color(rgb(color::BORDER_STRONG))
                    .rounded(RADIUS)
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(44.))
                            .px(px(16.))
                            .border_b_1()
                            .border_color(rgb(color::BORDER))
                            .child(display(&format!("Filter by {}", facet.label()), 16.))
                            .child(
                                div()
                                    .font_family(font::MONO)
                                    .text_size(px(12.))
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .child("servers"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .p(px(12.))
                            .border_b_1()
                            .border_color(rgb(color::BORDER))
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.search).prefix(
                                        svg()
                                            .path("icons/search.svg")
                                            .size(px(14.))
                                            .text_color(rgb(color::TEXT_DIM)),
                                    ),
                                ),
                            )
                            .when(self.has_content(), |bar| {
                                let this = cx.entity().downgrade();
                                bar.child(
                                    switch("facet-installed-only", installed_only)
                                        .caption("Installed only")
                                        .on_toggle(move |on, _, cx| {
                                            this.update(cx, |this, cx| {
                                                this.installed_only = on;
                                                this.refilter(cx);
                                                cx.notify();
                                            })
                                            .ok();
                                        }),
                                )
                            }),
                    )
                    .child(rows)
                    .when(count == 0, |card| {
                        card.child(
                            div()
                                .absolute()
                                .top(px(200.))
                                .w_full()
                                .flex()
                                .justify_center()
                                .text_size(px(13.))
                                .text_color(rgb(color::TEXT_MUTED))
                                .child("Nothing matches that search."),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .h(px(52.))
                            .px(px(12.))
                            .border_t_1()
                            .border_color(rgb(color::BORDER))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .child(SharedString::from(match chosen {
                                        0 => format!("Any {}", facet.label().to_lowercase()),
                                        1 => "1 chosen".to_owned(),
                                        n => format!("{n} chosen, any of them"),
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .when(chosen > 0, |row| {
                                        row.child(button("facet-clear", "Clear").on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                for value in &mut this.values {
                                                    value.chosen = false;
                                                }
                                                this.refilter(cx);
                                                cx.emit(FacetEvent::Cleared(facet));
                                                cx.notify();
                                            }),
                                        ))
                                    })
                                    .child(button("facet-done", "Done").light().on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(FacetEvent::Dismissed)),
                                    )),
                            ),
                    ),
            )
    }
}

/// A square that is filled and ticked when `on`.
fn tick(on: bool) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(16.))
        .rounded(RADIUS)
        .border_1()
        .map(|square| {
            if on {
                square
                    .bg(rgb(color::ACCENT))
                    .border_color(rgb(color::ACCENT))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xFFFFFF))
                            .child("✓"),
                    )
            } else {
                square.border_color(rgb(color::BORDER_STRONG))
            }
        })
}
