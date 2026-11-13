//! A searchable list over the window, for choosing a car or a track.

use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, KeyDownEvent, MouseButton, Render, SharedString,
    Subscription, TestSupportExt as _, Window,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::*,
    px, rgb, svg, uniform_list,
};

use crate::{
    theme::{RADIUS, alpha, color},
    ui::{badge, display},
};

pub struct PickerItem {
    pub id: SharedString,
    pub title: SharedString,
    pub detail: SharedString,
    pub badge: SharedString,
    /// Lower-cased text the search matches against.
    pub haystack: String,
}

impl PickerItem {
    pub fn new(id: &str, title: &str, detail: String, badge: &str) -> Self {
        Self {
            haystack: format!("{title} {detail} {id}").to_lowercase(),
            id: SharedString::from(id.to_owned()),
            title: SharedString::from(title.to_owned()),
            detail: detail.into(),
            badge: SharedString::from(badge.to_owned()),
        }
    }
}

pub enum PickerEvent {
    Picked(SharedString),
    Dismissed,
}

pub struct Picker {
    title: SharedString,
    items: Vec<PickerItem>,
    /// Indices into `items` that match the search, in order.
    matches: Vec<usize>,
    current: SharedString,
    search: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Picker {
    pub fn new(
        title: impl Into<SharedString>,
        items: Vec<PickerItem>,
        current: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let _subscription =
            cx.subscribe_in(&search, window, |this, search, event, _, cx| match event {
                InputEvent::Change => {
                    this.filter(&search.read(cx).value());
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if let Some(&first) = this.matches.first() {
                        cx.emit(PickerEvent::Picked(this.items[first].id.clone()));
                    }
                }
                _ => {}
            });
        search.update(cx, |search, cx| search.focus(window, cx));
        Self {
            title: title.into(),
            matches: (0..items.len()).collect(),
            items,
            current: SharedString::from(current.to_owned()),
            search,
            _subscription,
        }
    }

    /// Keeps the items containing every word of `query`.
    fn filter(&mut self, query: &str) {
        let query = query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| words.iter().all(|word| item.haystack.contains(word)))
            .map(|(index, _)| index)
            .collect();
    }
}

impl Render for Picker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.matches.len();
        let rows = uniform_list(
            "picker-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|row| {
                        let item = &this.items[this.matches[row]];
                        let id = item.id.clone();
                        let current = item.id == this.current;
                        div()
                            .id(row)
                            .flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .h(px(52.))
                            .px(px(16.))
                            .border_b_1()
                            .border_color(rgb(color::BORDER))
                            .cursor_pointer()
                            .when(current, |row| row.bg(rgb(color::ACCENT_WASH)))
                            .hover(|style| style.bg(rgb(color::RAISED)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(PickerEvent::Picked(id.clone()));
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .text_color(rgb(color::TEXT))
                                            .truncate()
                                            .child(item.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(color::TEXT_MUTED))
                                            .truncate()
                                            .child(item.detail.clone()),
                                    ),
                            )
                            .child(badge(&item.badge))
                            .test_support()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h(px(52. * 8.));

        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(96.))
            .bg(alpha(0x000000, 0.6))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PickerEvent::Dismissed)),
            )
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(PickerEvent::Dismissed);
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
                            .child(display(&self.title, 16.))
                            .child(
                                div()
                                    .font_family(crate::theme::font::MONO)
                                    .text_size(px(12.))
                                    .text_color(rgb(color::TEXT_MUTED))
                                    .child(format!("{count} of {}", self.items.len())),
                            ),
                    )
                    .child(
                        div()
                            .p(px(12.))
                            .border_b_1()
                            .border_color(rgb(color::BORDER))
                            .child(
                                Input::new(&self.search).prefix(
                                    svg()
                                        .path("icons/search.svg")
                                        .size(px(14.))
                                        .text_color(rgb(color::TEXT_DIM)),
                                ),
                            ),
                    )
                    .child(rows)
                    .when(count == 0, |card| {
                        card.child(
                            div()
                                .absolute()
                                .bottom(px(190.))
                                .w_full()
                                .flex()
                                .justify_center()
                                .text_size(px(13.))
                                .text_color(rgb(color::TEXT_MUTED))
                                .child("Nothing matches that search."),
                        )
                    }),
            )
    }
}
