use gpui_kit::{Hsla, IntoElement, PathBuilder, Pixels, canvas, point, prelude::*};

/// A parallelogram filling its parent, leaning right by `lean` at the top.
/// The launcher's signature shape: the logo mark and the launch button.
pub fn slant(color: impl Into<Hsla>, lean: Pixels) -> impl IntoElement {
    let color = color.into();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mut path = PathBuilder::fill();
            path.move_to(point(bounds.left() + lean, bounds.top()));
            path.line_to(point(bounds.right(), bounds.top()));
            path.line_to(point(bounds.right() - lean, bounds.bottom()));
            path.line_to(point(bounds.left(), bounds.bottom()));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .size_full()
}
