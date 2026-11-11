pub mod directory;
pub mod my;
pub mod reader;
pub mod static_pages;
pub mod submit;

use crate::state::AppState;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, Request, header},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use tower_http::{compression::CompressionLayer, services::ServeDir, trace::TraceLayer};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(directory::index))
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
        .route("/terms", get(static_pages::terms))
        .route("/privacy", get(static_pages::privacy))
        .route("/healthz", get(static_pages::healthz))
        .nest_service("/static", ServeDir::new("static").precompressed_gzip())
        .layer(middleware::from_fn(cache_static))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .fallback(static_pages::not_found)
        .with_state(state)
}

async fn cache_static(request: Request<axum::body::Body>, next: Next) -> Response {
    let is_static = request.uri().path().starts_with("/static/");
    let mut response = next.run(request).await;
    if is_static {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=86400"),
        );
    }
    response
}
