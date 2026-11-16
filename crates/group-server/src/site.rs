//! The page a person sees when they open the group server's address in a
//! browser: what Muretto is and where to get it. Everything it needs is
//! compiled in, so the server still makes no connections of its own; the
//! downloads point at the latest GitHub release.

use axum::{
    Router,
    http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HeaderValue},
    response::IntoResponse,
    routing::get,
};

const PAGE: &str = include_str!("landing.html");
const SCREENSHOT: &[u8] = include_bytes!("../../../docs/screenshots/drive.png");
const ICON: &[u8] = include_bytes!("../../../crates/app/assets/icon/muretto.svg");
const DISPLAY_FONT: &[u8] =
    include_bytes!("../../../crates/app/assets/fonts/BarlowCondensed-SemiBold.ttf");

/// Only what the page itself serves; inline styles, no scripts.
const POLICY: &str = "default-src 'none'; img-src 'self'; font-src 'self'; \
                      style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; \
                      frame-ancestors 'none'";

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/",
            get(|| async {
                (
                    [
                        (CONTENT_TYPE, "text/html; charset=utf-8"),
                        (CONTENT_SECURITY_POLICY, POLICY),
                        (CACHE_CONTROL, "public, max-age=300"),
                    ],
                    PAGE,
                )
            }),
        )
        .route("/drive.png", get(|| asset("image/png", SCREENSHOT)))
        .route("/icon.svg", get(|| asset("image/svg+xml", ICON)))
        .route("/display.ttf", get(|| asset("font/ttf", DISPLAY_FONT)))
}

async fn asset(content_type: &'static str, body: &'static [u8]) -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            ),
        ],
        body,
    )
}
