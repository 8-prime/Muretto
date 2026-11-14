//! Long work on the game folder, done off the interface thread while the
//! interface shows how far it has got.

use std::{sync::Arc, time::Duration};

use gpui_kit::{Context, Div, FontWeight, SharedString, div, prelude::*, px, relative, rgb};
use muretto_core::{
    Install, Result,
    packages::{Progress, Stage},
};

use crate::{
    theme::color,
    ui::{button, mono},
};

pub struct Job {
    pub title: SharedString,
    pub progress: Arc<Progress>,
}

impl Job {
    /// Runs `work` in the background and hands its result to `done` on the
    /// view. The view is redrawn several times a second until then, so a
    /// [`Job::strip`] in it stays current.
    pub fn start<V: 'static, T: Send + 'static>(
        title: impl Into<SharedString>,
        install: Install,
        cx: &mut Context<V>,
        work: impl FnOnce(&Install, &Progress) -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut V, Result<T>, &mut Context<V>) + 'static,
    ) -> Self {
        let progress = Progress::new();
        let working = cx.background_executor().spawn({
            let progress = progress.clone();
            async move { work(&install, &progress) }
        });
        cx.spawn(async move |this, cx| {
            let result = working.await;
            this.update(cx, |this, cx| {
                done(this, result, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();

        // The view drops its `Job` when the work is done; this loop holds the
        // last reference then, and stops.
        let watched = progress.clone();
        cx.spawn(async move |this, cx| {
            while Arc::strong_count(&watched) > 1 {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            title: title.into(),
            progress,
        }
    }

    /// The job's title, what it is doing, a bar, and a way to stop it while
    /// stopping is still safe.
    pub fn strip(&self) -> Div {
        let progress = &self.progress;
        let megabytes = |bytes: u64| format!("{:.1}", bytes as f64 / 1e6);
        let detail = match (progress.stage(), progress.total()) {
            (Stage::Starting, _) => "Starting".to_owned(),
            (Stage::Downloading, Some(total)) => format!(
                "Downloading · {} of {} MB",
                megabytes(progress.done()),
                megabytes(total)
            ),
            (Stage::Downloading, None) => {
                format!("Downloading · {} MB", megabytes(progress.done()))
            }
            (Stage::Unpacking, _) => "Unpacking".to_owned(),
            (Stage::Installing, _) => "Moving files into the game folder".to_owned(),
        };
        let cancellable = progress.stage() != Stage::Installing;
        let cancel = progress.clone();
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
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
                            .gap(px(10.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.title.clone()),
                            )
                            .child(mono(detail, 12.5).text_color(rgb(color::TEXT_MUTED))),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(rgb(color::BORDER))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(progress.fraction().unwrap_or(0.)))
                                    .rounded_full()
                                    .bg(rgb(color::ACCENT)),
                            ),
                    ),
            )
            .when(cancellable, |strip| {
                strip.child(button("job-cancel", "Cancel").on_click(move |_, _, _| cancel.cancel()))
            })
    }
}
