//! Form fields bound to a view: each owns the state its control needs and
//! reports edits back through a typed callback.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    App, Context, Entity, IntoElement, SharedString, Subscription, Window,
    base::{Slider, SliderIndicator, SliderThumb, SliderTrack},
    component::{
        IndexPath,
        input::{Input, InputEvent, InputState},
        select::{SearchableVec, Select, SelectEvent, SelectState},
        slider::{SliderEvent, SliderState},
    },
    div,
    prelude::*,
    px, relative, rgb,
};

use crate::theme::{color, font};

type Options = SearchableVec<SharedString>;

/// A drop-down choosing one of a list of values.
pub struct ChoiceField<T> {
    state: Entity<SelectState<Options>>,
    values: Rc<RefCell<Vec<T>>>,
    _subscription: Subscription,
}

impl<T: Clone + PartialEq + 'static> ChoiceField<T> {
    pub fn new<V: 'static>(
        options: Vec<(T, SharedString)>,
        selected: &T,
        window: &mut Window,
        cx: &mut Context<V>,
        on_change: impl Fn(&mut V, T, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        let (values, labels): (Vec<T>, Vec<SharedString>) = options.into_iter().unzip();
        let index = values
            .iter()
            .position(|v| v == selected)
            .map(IndexPath::new);
        let state = cx.new(|cx| SelectState::new(SearchableVec::new(labels), index, window, cx));
        let values = Rc::new(RefCell::new(values));
        let _subscription = cx.subscribe_in(&state, window, {
            let values = values.clone();
            move |view, state, _: &SelectEvent<Options>, window, cx| {
                let Some(index) = state.read(cx).selected_index(cx) else {
                    return;
                };
                let value = values.borrow().get(index.row).cloned();
                if let Some(value) = value {
                    on_change(view, value, window, cx);
                }
            }
        });
        Self {
            state,
            values,
            _subscription,
        }
    }

    /// Replaces the list, for options that depend on other choices.
    pub fn set_options(
        &self,
        options: Vec<(T, SharedString)>,
        selected: &T,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (values, labels): (Vec<T>, Vec<SharedString>) = options.into_iter().unzip();
        let index = values
            .iter()
            .position(|v| v == selected)
            .map(IndexPath::new);
        *self.values.borrow_mut() = values;
        self.state.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(labels), window, cx);
            state.set_selected_index(index, window, cx);
        });
    }

    pub fn select(&self, value: &T, window: &mut Window, cx: &mut App) {
        let index = self
            .values
            .borrow()
            .iter()
            .position(|v| v == value)
            .map(IndexPath::new);
        self.state.update(cx, |state, cx| {
            if state.selected_index(cx) != index {
                state.set_selected_index(index, window, cx);
            }
        });
    }

    pub fn element(&self) -> impl IntoElement + use<T> {
        Select::new(&self.state).h(px(36.)).bg(rgb(color::BG))
    }

    /// The drop-down, showing `placeholder` while nothing is chosen.
    pub fn element_or(&self, placeholder: &'static str) -> impl IntoElement + use<T> {
        Select::new(&self.state)
            .placeholder(placeholder)
            .h(px(36.))
            .bg(rgb(color::BG))
    }
}

/// A slider over a numeric range.
pub struct RangeField {
    state: Entity<SliderState>,
    _subscription: Subscription,
}

impl RangeField {
    pub fn new<V: 'static>(
        (min, max, step): (f32, f32, f32),
        value: f32,
        cx: &mut Context<V>,
        on_change: impl Fn(&mut V, f32, &mut Context<V>) + 'static,
    ) -> Self {
        let state = cx.new(|_| {
            SliderState::new()
                .min(min)
                .max(max)
                .step(step)
                .default_value(value)
        });
        let _subscription = cx.subscribe(&state, move |view, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                on_change(view, value.end(), cx);
            }
        });
        Self {
            state,
            _subscription,
        }
    }

    /// Moves the handle to `value`, unless it is already within half a step
    /// of it, so a drag in progress is left alone.
    pub fn set(&self, value: f32, window: &mut Window, cx: &mut App) {
        let state = self.state.read(cx);
        let (current, step) = (state.value().end(), state.step_value());
        let value = value.clamp(state.min_value(), state.max_value());
        if (current - value).abs() > step / 2. {
            self.state
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    /// A light rail, filled red up to a round red handle.
    pub fn element(&self, name: &'static str, cx: &App) -> impl IntoElement + use<> {
        let filled = self.state.read(cx).percentage().end;
        Slider::new(&self.state)
            .accessibility_label(name)
            .flex()
            .items_center()
            .w_full()
            .child(
                SliderTrack::new(&self.state)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .w_full()
                    .h(px(18.))
                    // Keep the handle inside the field at both ends.
                    .px(px(7.))
                    .child(
                        SliderIndicator::new(&self.state)
                            .relative()
                            .w_full()
                            .h(px(6.))
                            .rounded_full()
                            .bg(rgb(color::LIGHT))
                            .child(
                                div()
                                    .absolute()
                                    .h_full()
                                    .left_0()
                                    .right(relative(1. - filled))
                                    .rounded_full()
                                    .bg(rgb(color::ACCENT)),
                            )
                            .child(
                                SliderThumb::new(&self.state)
                                    .absolute()
                                    .top(px(-4.))
                                    .left(relative(filled))
                                    .ml(px(-7.))
                                    .size(px(14.))
                                    .rounded_full()
                                    .bg(rgb(color::ACCENT)),
                            ),
                    ),
            )
    }
}

/// A box for a whole number.
pub struct NumberField {
    state: Entity<InputState>,
    _subscription: Subscription,
}

impl NumberField {
    pub fn new<V: 'static>(
        value: u32,
        window: &mut Window,
        cx: &mut Context<V>,
        on_change: impl Fn(&mut V, u32, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        let state = cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
        let _subscription = cx.subscribe_in(&state, window, {
            move |view, state, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value();
                let digits: String = text.chars().filter(char::is_ascii_digit).take(4).collect();
                if digits != text.as_ref() {
                    let digits = digits.clone();
                    state.update(cx, |state, cx| state.set_value(digits, window, cx));
                }
                if let Ok(number) = digits.parse() {
                    on_change(view, number, window, cx);
                }
            }
        });
        Self {
            state,
            _subscription,
        }
    }

    /// Shows `value`, unless the box already holds that number.
    pub fn set(&self, value: u32, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).value().parse() != Ok(value) {
            self.state.update(cx, |state, cx| {
                state.set_value(value.to_string(), window, cx);
            });
        }
    }

    pub fn element(&self, id: &'static str, enabled: bool) -> impl IntoElement + use<> {
        div()
            .font_family(font::MONO)
            .when(!enabled, |this| this.opacity(0.45))
            // `Input::h` is for multi-line inputs; the box height is a style.
            .child(Styled::h(
                Input::new(&self.state)
                    .id(id)
                    .font_family(font::MONO)
                    .bg(rgb(color::BG))
                    .text_size(px(15.))
                    .disabled(!enabled),
                px(36.),
            ))
    }
}
