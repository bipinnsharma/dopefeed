use crate::{
    error::{AppError, AppResult},
    models::{Post, TIMELINE_PAGE, TimelineQuery},
    state::{AppState, POST_COLUMNS},
};
use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};

#[derive(Template)]
#[template(path = "my.html")]
struct MyTemplate;

#[derive(Template)]
#[template(path = "_post_list.html")]
struct PostList {
    posts: Vec<Post>,
    page: i64,
    has_more: bool,
    feeds: String,
}

pub async fn my() -> AppResult<Html<String>> {
    Ok(Html(MyTemplate.render()?))
}

pub async fn timeline(
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> AppResult<Html<String>> {
    let raw = query.feeds.as_deref().unwrap_or("").trim();
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if parts.len() > 50 {
        return Err(AppError::BadRequest(
            "feeds must contain between 1 and 50 ids".into(),
        ));
    }
    let mut ids: Vec<i64> = Vec::new();
    for part in parts {
        let id = part.parse::<i64>().map_err(|_| {
            AppError::BadRequest("feeds must be a comma-separated list of ids".into())
        })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(AppError::BadRequest(
            "feeds must contain between 1 and 50 ids".into(),
        ));
    }

    let page = query.page.unwrap_or(1).max(1);
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT {POST_COLUMNS} FROM posts p JOIN feeds f ON f.id = p.feed_id
         WHERE f.status = 'active' AND p.feed_id IN ({placeholders})
         ORDER BY p.published_at DESC, p.id DESC LIMIT ? OFFSET ?"
    );

    let mut rows = sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(sql));
    for id in &ids {
        rows = rows.bind(id);
    }
    let posts = rows
        .bind(TIMELINE_PAGE + 1)
        .bind((page - 1) * TIMELINE_PAGE)
        .fetch_all(&state.pool)
        .await?;
    let has_more = posts.len() as i64 > TIMELINE_PAGE;
    let posts = posts.into_iter().take(TIMELINE_PAGE as usize).collect();
    let feeds = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");

    Ok(Html(
        PostList {
            posts,
            page,
            has_more,
            feeds,
        }
        .render()?,
    ))
}
