use crate::{
    error::{AppError, AppResult},
    models::CATEGORIES,
    routes::submit::{self, CategoryOption, SubmitForm},
    state::AppState,
    worker::discover,
};
use askama::Template;
use axum::{
    extract::{ConnectInfo, Form, Path, Query, State},
    http::{
        HeaderMap, HeaderValue, Request, StatusCode,
        header::{AUTHORIZATION, WWW_AUTHENTICATE},
    },
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::Engine;
use serde::Deserialize;
use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

const MAX_FAILURES: usize = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(300);

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminPage {
    feeds: Vec<AdminFeed>,
    categories: Vec<CategoryOption>,
    url: String,
    title: String,
    error: Option<String>,
    flash: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AdminFeed {
    id: i64,
    title: String,
    domain: String,
    category: String,
    status: String,
    post_count: i64,
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminQuery {
    msg: Option<String>,
}

pub async fn page(
    State(state): State<AppState>,
    Query(query): Query<AdminQuery>,
) -> AppResult<Html<String>> {
    render(
        &state,
        "",
        String::new(),
        String::new(),
        None,
        flash(query.msg.as_deref()),
    )
    .await
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<SubmitForm>,
) -> AppResult<Response> {
    if state.config.read_only {
        return Err(AppError::Forbidden(
            "Admin changes are disabled on this read-only instance.".into(),
        ));
    }
    if !submit::origin_allowed(&headers, &state.config.base_url) {
        return Err(AppError::Forbidden(
            "Submit from the site itself, not another origin.".into(),
        ));
    }
    let category = form.category.trim().to_ascii_lowercase();
    let title = form
        .title
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect::<String>();

    let Some(input_url) = submit::parse_input(&form.url) else {
        return invalid(
            &state,
            &form,
            &category,
            &title,
            "Enter a valid http(s) address.",
        )
        .await;
    };
    if !CATEGORIES.contains(&category.as_str()) {
        return invalid(&state, &form, &category, &title, "Pick a category.").await;
    }

    let resolved = match discover::resolve(&state.http, &input_url).await {
        Ok(resolved) => resolved,
        Err(message) => return invalid(&state, &form, &category, &title, &message).await,
    };

    match submit::insert_resolved(&state, resolved, &category, Some(&title)).await? {
        Some(_) => Ok(Redirect::to("/admin?msg=added").into_response()),
        None => {
            invalid(
                &state,
                &form,
                &category,
                &title,
                "That feed is already in the directory.",
            )
            .await
        }
    }
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Response> {
    if state.config.read_only {
        return Err(AppError::Forbidden(
            "Admin changes are disabled on this read-only instance.".into(),
        ));
    }
    if !submit::origin_allowed(&headers, &state.config.base_url) {
        return Err(AppError::Forbidden(
            "Delete from the site itself, not another origin.".into(),
        ));
    }
    sqlx::query("DELETE FROM feeds WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/admin?msg=deleted").into_response())
}

async fn invalid(
    state: &AppState,
    form: &SubmitForm,
    category: &str,
    title: &str,
    error: &str,
) -> AppResult<Response> {
    let page = render(
        state,
        category,
        form.url.trim().to_string(),
        title.to_string(),
        Some(error.into()),
        None,
    )
    .await?;
    Ok((StatusCode::BAD_REQUEST, page).into_response())
}

fn flash(msg: Option<&str>) -> Option<String> {
    match msg {
        Some("added") => Some("Feed added. Posts will appear as they are fetched.".into()),
        Some("deleted") => Some("Feed and its posts deleted.".into()),
        _ => None,
    }
}

async fn render(
    state: &AppState,
    category: &str,
    url: String,
    title: String,
    error: Option<String>,
    flash: Option<String>,
) -> AppResult<Html<String>> {
    let feeds = sqlx::query_as::<_, AdminFeed>(
        "SELECT f.id, f.title, f.domain, f.category, f.status,
         (SELECT COUNT(*) FROM posts p WHERE p.feed_id = f.id) AS post_count,
         f.last_error
         FROM feeds f ORDER BY f.id DESC LIMIT 500",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(
        AdminPage {
            feeds,
            categories: submit::category_options(category),
            url,
            title,
            error,
            flash,
        }
        .render()?,
    ))
}

/// Basic auth with `ADMIN_KEY` as the password. Runs before every admin route.
pub async fn auth(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let Some(key) = state.config.admin_key.as_deref() else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    // Browsers probe without credentials first; only count real key attempts.
    if request.headers().contains_key(AUTHORIZATION) {
        let now = Instant::now();
        if locked_out(&state, addr.ip(), now) {
            return (StatusCode::TOO_MANY_REQUESTS, "Too many attempts.").into_response();
        }
        if authorized(request.headers(), key) {
            return next.run(request).await;
        }
        record_failure(&state, addr.ip(), now);
    }
    (
        StatusCode::UNAUTHORIZED,
        [(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"dopefeed admin\", charset=\"UTF-8\""),
        )],
        "Authentication required",
    )
        .into_response()
}

fn authorized(headers: &HeaderMap, key: &str) -> bool {
    let Some(value) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(encoded) = value.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(pair) = String::from_utf8(decoded) else {
        return false;
    };
    let Some((_, password)) = pair.split_once(':') else {
        return false;
    };
    constant_time_eq(password.as_bytes(), key.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn locked_out(state: &AppState, ip: IpAddr, now: Instant) -> bool {
    let Ok(mut attempts) = state.admin_attempts.lock() else {
        return true;
    };
    let entries = attempts.entry(ip).or_default();
    entries.retain(|at| now.duration_since(*at) < FAILURE_WINDOW);
    entries.len() >= MAX_FAILURES
}

fn record_failure(state: &AppState, ip: IpAddr, now: Instant) {
    if let Ok(mut attempts) = state.admin_attempts.lock() {
        attempts.entry(ip).or_default().push(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_auth_compares_password() {
        let headers = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
            headers
        };
        let ok = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("admin:ab12cd")
        );
        assert!(authorized(&headers(&ok), "ab12cd"));
        assert!(!authorized(&headers(&ok), "ab12ce"));
        assert!(!authorized(&headers("Basic !!!"), "ab12cd"));
        assert!(!authorized(&HeaderMap::new(), "ab12cd"));
    }
}
