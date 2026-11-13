use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, Stateful,
    TestSupportExt as _, Window, div, prelude::*, px, rgb,
};

use super::{display, slant, tracked};
use crate::theme::{RADIUS, color, font};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type ToggleHandler = Box<dyn Fn(bool, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, PartialEq)]
enum ButtonKind {
    /// Outlined, on a panel or bar.
    Quiet,
    /// Light fill: the main action of a group.
    Light,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    kind: ButtonKind,
    on_click: Option<ClickHandler>,
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button {
        id: id.into(),
        label: label.into(),
        kind: ButtonKind::Quiet,
        on_click: None,
    }
}

impl Button {
    pub fn light(mut self) -> Self {
        self.kind = ButtonKind::Light;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (bg, hover, text, border) = match self.kind {
            ButtonKind::Quiet => (
                color::SURFACE,
                color::RAISED,
                color::TEXT,
                color::BORDER_STRONG,
            ),
            ButtonKind::Light => (color::LIGHT, 0xFFFFFF, color::BG, color::LIGHT),
        };
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(32.))
            .px(px(12.))
            .bg(rgb(bg))
            .border_1()
            .border_color(rgb(border))
            .rounded(RADIUS)
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(text))
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(hover)))
            .when_some(self.on_click, |this, handler| this.on_click(handler))
            .child(self.label)
            .test_support()
    }
}

/// The slanted red launch button.
#[derive(IntoElement)]
pub struct LaunchButton {
    id: ElementId,
    label: SharedString,
    enabled: bool,
    on_click: Option<ClickHandler>,
}

pub fn launch_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> LaunchButton {
    LaunchButton {
        id: id.into(),
        label: label.into(),
        enabled: true,
        on_click: None,
    }
}

impl LaunchButton {
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for LaunchButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let lean = px(12.);
        let base = div()
            .id(self.id)
            .group("launch")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(152.))
            .h(px(42.));
        if !self.enabled {
            return base
                .child(slant(rgb(color::CHIP), lean))
                .child(
                    display(&self.label, 24.)
                        .relative()
                        .text_color(rgb(color::TEXT_DIM)),
                )
                .test_support();
        }
        base.cursor_pointer()
            .child(slant(rgb(color::ACCENT), lean))
            .child(
                div()
                    .absolute()
                    .size_full()
                    .invisible()
                    .group_hover("launch", |style| style.visible())
                    .child(slant(rgb(color::ACCENT_HOVER), lean)),
            )
            .child(
                display(&self.label, 24.)
                    .relative()
                    .text_color(rgb(0xFFFFFF)),
            )
            .when_some(self.on_click, |this, handler| this.on_click(handler))
            .test_support()
    }
}

/// An on/off switch with its caption to the right.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    on: bool,
    caption: Option<SharedString>,
    on_toggle: Option<ToggleHandler>,
}

pub fn switch(id: impl Into<ElementId>, on: bool) -> Switch {
    Switch {
        id: id.into(),
        on,
        caption: None,
        on_toggle: None,
    }
}

impl Switch {
    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Called with the state the switch should move to.
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on = self.on;
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .w(px(38.))
                    .h(px(20.))
                    .p(px(3.))
                    .rounded_full()
                    .bg(rgb(if on { color::ACCENT } else { color::SWITCH_OFF }))
                    .when(on, |track| track.justify_end())
                    .child(div().size(px(14.)).rounded_full().bg(rgb(if on {
                        0xFFFFFF
                    } else {
                        color::SWITCH_KNOB_OFF
                    }))),
            )
            .when_some(self.caption, |this, caption| {
                this.child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(color::TEXT))
                        .child(caption),
                )
            })
            .when_some(self.on_toggle, |this, handler| {
                this.on_click(move |_, window, cx| handler(!on, window, cx))
            })
            .test_support()
    }
}

/// One of a set of mutually exclusive tiles; the chosen one is filled red.
pub fn choice_tile(id: impl Into<ElementId>, text: &str, chosen: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .h(px(44.))
        .rounded(RADIUS)
        .border_1()
        .cursor_pointer()
        .map(|tile| {
            if chosen {
                tile.bg(rgb(color::ACCENT)).border_color(rgb(color::ACCENT))
            } else {
                tile.bg(rgb(color::BG))
                    .border_color(rgb(color::BORDER))
                    .hover(|style| style.bg(rgb(color::RAISED)))
            }
        })
        .child(display(text, 16.))
}

/// A small outlined tag: where content came from, or its state.
pub fn badge(text: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(20.))
        .px(px(6.))
        .border_1()
        .border_color(rgb(color::BORDER_STRONG))
        .rounded(RADIUS)
        .child(
            tracked(&text.to_uppercase(), px(10.), 0.12)
                .font_family(font::BODY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color::TEXT_MUTED)),
        )
}
