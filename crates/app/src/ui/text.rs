use gpui_kit::{Div, FontWeight, Pixels, SharedString, div, prelude::*, px, relative, rgb};

use crate::theme::{color, font};

/// Text with extra space between its letters.
///
/// The UI framework has no letter-spacing, so each character is laid out as
/// its own flex item. That costs kerning and wrapping, neither of which
/// tracked capitals want; do not use this for running text.
pub fn tracked(text: &str, size: Pixels, tracking_em: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(size * tracking_em)
        .text_size(size)
        .line_height(relative(1.2))
        .whitespace_nowrap()
        .children(text.chars().map(|c| {
            if c == ' ' {
                div().w(size * 0.26).into_any_element()
            } else {
                SharedString::from(c.to_string()).into_any_element()
            }
        }))
}

/// Bold italic condensed capitals: headings, navigation, the launch button.
pub fn display(text: &str, size: f32) -> Div {
    tracked(&text.to_uppercase(), px(size), 0.045)
        .font_family(font::DISPLAY)
        .font_weight(FontWeight::BOLD)
        .italic()
        .text_color(rgb(color::TEXT))
}

/// Small tracked capitals naming a field or a figure.
pub fn label(text: &str) -> Div {
    tracked(&text.to_uppercase(), px(11.), 0.1)
        .font_family(font::BODY)
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color::TEXT_MUTED))
}

/// A figure in the monospaced face.
pub fn mono(text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .font_family(font::MONO)
        .text_size(px(size))
        .line_height(relative(1.25))
        .text_color(rgb(color::TEXT))
        .whitespace_nowrap()
        .child(text.into())
}

/// Secondary running text.
pub fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .line_height(relative(1.35))
        .text_color(rgb(color::TEXT_MUTED))
        .child(text.into())
}

/// A label above a figure.
pub fn stat(name: &str, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(label(name))
        .child(mono(value, 14.))
}
