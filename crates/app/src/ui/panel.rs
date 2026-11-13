use gpui_kit::{
    AnyElement, App, Div, IntoElement, RenderOnce, SharedString, StyleRefinement, Window,
    component::StyledExt as _, div, prelude::*, px, rgb,
};

use super::{display, label, mono};
use crate::theme::{RADIUS, color};

/// A titled, bordered section of a screen.
#[derive(IntoElement)]
pub struct Panel {
    title: SharedString,
    action: Option<AnyElement>,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

pub fn panel(title: impl Into<SharedString>) -> Panel {
    Panel {
        title: title.into(),
        action: None,
        children: Vec::new(),
        style: StyleRefinement::default(),
    }
}

impl Panel {
    /// A control at the right end of the title bar.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl ParentElement for Panel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Panel {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Panel {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .bg(rgb(color::SURFACE))
            .border_1()
            .border_color(rgb(color::BORDER))
            .rounded(RADIUS)
            .overflow_hidden()
            .refine_style(&self.style)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .h(px(44.))
                    .pl(px(16.))
                    .pr(px(6.))
                    .border_b_1()
                    .border_color(rgb(color::BORDER))
                    .child(display(&self.title, 16.))
                    .children(self.action),
            )
            .children(self.children)
    }
}

/// Figures side by side in one bordered strip, each under its label.
pub fn stat_strip(stats: impl IntoIterator<Item = (&'static str, SharedString)>) -> Div {
    let mut strip = div()
        .flex()
        .border_1()
        .border_color(rgb(color::BORDER))
        .rounded(RADIUS);
    for (index, (name, value)) in stats.into_iter().enumerate() {
        strip = strip.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .px(px(10.))
                .py(px(8.))
                .when(index > 0, |cell| {
                    cell.border_l_1().border_color(rgb(color::BORDER))
                })
                .child(label(name))
                .child(mono(value, 14.)),
        );
    }
    strip
}
