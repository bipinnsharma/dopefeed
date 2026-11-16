use crate::{
    error::{AppError, AppResult},
    models::CATEGORIES,
    state::AppState,
    worker::{
        self,
        discover::{self, Resolved},
    },
};
use askama::Template;
use axum::{
    extract::{ConnectInfo, Form, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};
use url::Url;

const RATE_LIMIT: usize = 5;
const RATE_WINDOW: Duration = Duration::from_secs(3600);

#[derive(Template)]
#[template(path = "submit.html")]
struct SubmitPage {
    error: Option<String>,
    url: String,
    title: String,
    categories: Vec<CategoryOption>,
}

pub(crate) struct CategoryOption {
    pub(crate) name: &'static str,
    pub(crate) selected: bool,
}

pub(crate) fn category_options(selected: &str) -> Vec<CategoryOption> {
    CATEGORIES
        .iter()
        .map(|name| CategoryOption {
            name,
            selected: *name == selected,
        })
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct SubmitForm {
    pub url: String,
    pub category: String,
    pub title: Option<String>,
}

pub async fn form() -> AppResult<Html<String>> {
    Ok(Html(
        SubmitPage {
            error: None,
            url: String::new(),
            title: String::new(),
            categories: category_options(""),
        }
        .render()?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SubmitForm>,
) -> AppResult<Response> {
    if !origin_allowed(&headers, &state.config.base_url) {
        return Err(AppError::Forbidden(
            "Submit from the site itself, not another origin.".into(),
        ));
    }
    check_rate_limit(&state, addr.ip())?;

    let category = form.category.trim().to_ascii_lowercase();
    let title = form
        .title
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect::<String>();

    let Some(input_url) = parse_input(&form.url) else {
        return Ok(invalid(
            &form,
            &category,
            &title,
            "Enter a valid http(s) address.",
        ));
    };
    if !CATEGORIES.contains(&category.as_str()) {
        return Ok(invalid(&form, &category, &title, "Pick a category."));
    }

    let resolved = match discover::resolve(&state.http, &input_url).await {
        Ok(resolved) => resolved,
        Err(message) => return Ok(invalid(&form, &category, &title, &message)),
    };

    match insert_resolved(&state, resolved, &category, Some(&title)).await? {
        Some(id) => Ok(Redirect::to(&format!("/f/{id}?msg=added")).into_response()),
        None => Ok(invalid(
            &form,
            &category,
            &title,
            "That feed is already in the directory.",
        )),
    }
}

fn invalid(form: &SubmitForm, category: &str, title: &str, error: &str) -> Response {
    let page = SubmitPage {
        error: Some(error.to_string()),
        url: form.url.trim().to_string(),
        title: title.to_string(),
        categories: category_options(category),
    };
    match page.render() {
        Ok(html) => (StatusCode::BAD_REQUEST, Html(html)).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Error").into_response(),
    }
}

pub(crate) fn parse_input(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2048 {
        return None;
    }
    let parsed = Url::parse(raw)
        .ok()
        .or_else(|| Url::parse(&format!("https://{raw}")).ok())?;
    matches!(parsed.scheme(), "http" | "https").then_some(parsed)
}

fn check_rate_limit(state: &AppState, ip: IpAddr) -> AppResult<()> {
    let now = Instant::now();
    let mut attempts = state
        .submissions
        .lock()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("rate limit lock poisoned")))?;
    let entries = attempts.entry(ip).or_default();
    entries.retain(|at| now.duration_since(*at) < RATE_WINDOW);
    if entries.len() >= RATE_LIMIT {
        return Err(AppError::TooManyRequests(
            "Too many submissions. Try again later.".into(),
        ));
    }
    entries.push(now);
    Ok(())
}

pub(crate) fn origin_allowed(headers: &HeaderMap, base: &Url) -> bool {
    let source = headers
        .get(header::ORIGIN)
        .or_else(|| headers.get(header::REFERER))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Url::parse(value).ok());
    let Some(source) = source else {
        return false;
    };
    source.host_str() == base.host_str()
        && source.port_or_known_default() == base.port_or_known_default()
}

/// Inserts a resolved feed, fetches its entries immediately, and returns its id.
pub async fn insert_resolved(
    state: &AppState,
    resolved: Resolved,
    category: &str,
    title_override: Option<&str>,
) -> anyhow::Result<Option<i64>> {
    let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM feeds WHERE url = ?")
        .bind(&resolved.feed_url)
        .fetch_optional(&state.pool)
        .await?;
    if existing.is_some() {
        return Ok(None);
    }

    let domain = domain_of(&resolved.site_url).unwrap_or_default();
    let title = title_override
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .or_else(|| resolved.title.clone())
        .unwrap_or_else(|| domain.clone());
    // A synthetic feed's ETag belongs to the page, not a feed; storing it would
    // make the next refresh a 304 and skip discovery.
    let (etag, last_modified) = if resolved.synthetic {
        (None, None)
    } else {
        (resolved.etag.clone(), resolved.last_modified.clone())
    };
    let images_fetched_at = resolved
        .images_fetched
        .then(|| chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());
    let avatar_url =
        crate::assets::cache_or_keep(&state.http, resolved.avatar_url.clone(), "feeds").await;
    let banner_url =
        crate::assets::cache_or_keep(&state.http, resolved.banner_url.clone(), "banners").await;

    let result = sqlx::query(
        "INSERT INTO feeds (url, site_url, domain, title, description, category, status,
         avatar_url, banner_url, images_fetched_at, etag, last_modified, last_fetched_at, next_fetch_at)
         VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, ?, datetime('now'), datetime('now'))",
    )
    .bind(&resolved.feed_url)
    .bind(&resolved.site_url)
    .bind(&domain)
    .bind(&title)
    .bind(resolved.description.clone())
    .bind(category)
    .bind(avatar_url)
    .bind(banner_url)
    .bind(images_fetched_at)
    .bind(etag)
    .bind(last_modified)
    .execute(&state.pool)
    .await?;
    let id = result.last_insert_rowid();
    worker::fetch::ingest(&state.pool, id, &resolved.feed, &resolved.site_url).await?;
    crate::assets::localize_post_thumbs(
        &state.pool,
        &state.http,
        std::path::Path::new(crate::assets::DIR),
        id,
    )
    .await;
    Ok(Some(id))
}

pub fn domain_of(url: &str) -> Option<String> {
    let host = Url::parse(url).ok()?.host_str()?.to_string();
    Some(host.trim_start_matches("www.").to_string())
}
