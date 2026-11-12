use crate::{
    error::{AppError, AppResult},
    state::AppState,
};
use askama::Template;
use axum::{extract::State, response::Html};

#[derive(Template)]
#[template(path = "about.html")]
struct InfoPage {
    title: &'static str,
    body: &'static str,
}

const ABOUT: &str = "<p>dopefeed is a public directory of RSS, Atom and YouTube feeds, \
paired with a clean reader. No accounts, no tracking cookies, no algorithms — just the \
feeds people actually read.</p><p>Subscriptions and read state live in your browser's \
localStorage. Nothing about your reading is sent to the server.</p>\
<p>Feeds are fetched by a small background worker. New submissions are validated and \
added automatically.</p>";

const TERMS: &str = "<p>Use dopefeed lawfully. Do not submit feeds you do not have the \
right to share, and do not attempt to disrupt the service.</p><p>The directory is \
provided as-is, without warranty. Content belongs to its authors; dopefeed only links \
to it.</p>";

const PRIVACY: &str = "<p>dopefeed sets no cookies and runs no analytics. Subscriptions \
and read state stay in your browser's localStorage.</p><p>The server logs standard \
request metadata (method, path, status) for operation and abuse prevention. Submitted \
feeds and their public content are stored on the server.</p>";

const WEBSITES: &str = "<p>A hand-picked roll of the sites behind the feeds — personal \
blogs, studios, and publications worth visiting directly.</p>\
<p class=\"meta\">Curated websites are coming soon.</p>";

const LINKS: &str = "<p>A running list of good corners of the web: tools, essays, and \
rabbit holes worth keeping.</p><p class=\"meta\">Curated links are coming soon.</p>";

const SKILLS: &str = "<p>Small, reusable workflows for reading and publishing on the \
open web.</p><p class=\"meta\">Skills are coming soon.</p>";

pub async fn about() -> AppResult<Html<String>> {
    Ok(Html(render("About", ABOUT)?))
}

pub async fn terms() -> AppResult<Html<String>> {
    Ok(Html(render("Terms", TERMS)?))
}

pub async fn privacy() -> AppResult<Html<String>> {
    Ok(Html(render("Privacy", PRIVACY)?))
}

pub async fn websites() -> AppResult<Html<String>> {
    Ok(Html(render("Websites", WEBSITES)?))
}

pub async fn links() -> AppResult<Html<String>> {
    Ok(Html(render("Links", LINKS)?))
}

pub async fn skills() -> AppResult<Html<String>> {
    Ok(Html(render("Skills", SKILLS)?))
}

fn render(title: &'static str, body: &'static str) -> AppResult<String> {
    Ok(InfoPage { title, body }.render()?)
}

pub async fn healthz(State(state): State<AppState>) -> AppResult<&'static str> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok("ok")
}

pub async fn not_found() -> AppError {
    AppError::NotFound
}
