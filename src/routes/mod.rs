pub mod admin;
pub mod directory;
pub mod my;
pub mod reader;
pub mod static_pages;
pub mod submit;

use crate::state::AppState;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use tower_http::{compression::CompressionLayer, services::ServeDir, trace::TraceLayer};

pub fn router(state: AppState) -> Router {
    let admin_routes = Router::new()
        .route("/admin", get(admin::page))
        .route("/admin/feeds", post(admin::create))
        .route("/admin/feeds/{id}/delete", post(admin::delete))
        .route_layer(middleware::from_fn_with_state(state.clone(), admin::auth));

    Router::new()
        .route("/", get(directory::index))
        .merge(admin_routes)
        .route("/feeds", get(directory::grid))
        .route("/f/{id}", get(reader::detail))
        .route("/read/{id}", get(reader::read))
        .route("/my", get(my::my))
        .route("/api/timeline", get(my::timeline))
        .route(
            "/submit",
            get(submit::form)
                .post(submit::create)
                .layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route("/about", get(static_pages::about))
        .route("/websites", get(static_pages::websites))
        .route("/links", get(static_pages::links))
        .route("/skills", get(static_pages::skills))
        .route("/terms", get(static_pages::terms))
        .route("/privacy", get(static_pages::privacy))
        .route("/healthz", get(static_pages::healthz))
        .nest_service("/static", ServeDir::new("static"))
        .layer(middleware::from_fn(cache_static))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .fallback(static_pages::not_found)
        .with_state(state)
}

async fn cache_static(request: Request<axum::body::Body>, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let is_get = request.method() == Method::GET;
    let mut response = next.run(request).await;

    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );

    let ok = response.status() == StatusCode::OK;
    let policy = if path.starts_with("/static/assets/") {
        // A missing file must not be cached for a year.
        ok.then_some("public, max-age=31536000, immutable")
    } else if path.starts_with("/static/") {
        // Debug builds revalidate so CSS/JS edits show immediately.
        Some(if cfg!(debug_assertions) {
            "no-cache"
        } else {
            "public, max-age=86400"
        })
    } else if is_get && ok && path == "/api/timeline" {
        Some("public, s-maxage=60, stale-while-revalidate=300")
    } else if is_get && ok && is_public_page(&path) {
        Some("public, s-maxage=600, stale-while-revalidate=86400")
    } else {
        None
    };
    if let Some(policy) = policy {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(policy));
    }
    response
}

/// Public pages only: admin, submit, other API paths and health checks stay uncached.
fn is_public_page(path: &str) -> bool {
    matches!(
        path,
        "/" | "/feeds" | "/about" | "/websites" | "/links" | "/skills" | "/terms" | "/privacy"
    ) || path.starts_with("/f/")
        || path.starts_with("/read/")
}
