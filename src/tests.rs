use crate::{
    assets,
    config::Config,
    models::{Post, PostRow},
    routes,
    state::AppState,
    worker::fetch,
    worker::fetch::DueFeed,
    worker::ssrf,
};
use reqwest::{
    Client,
    header::{CONTENT_TYPE, ORIGIN},
    redirect::Policy,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{net::SocketAddr, str::FromStr};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const RSS: &[u8] = include_bytes!("../fixtures/rss2.xml");
const ATOM: &[u8] = include_bytes!("../fixtures/atom.xml");
const YOUTUBE: &[u8] = include_bytes!("../fixtures/youtube.xml");
const MALFORMED: &[u8] = include_bytes!("../fixtures/malformed.xml");

async fn test_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

fn test_config(base_url: &str) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        base_url: base_url.parse().unwrap(),
        database_url: "sqlite::memory:".into(),
        fetch_concurrency: 2,
        admin_key: None,
    }
}

async fn test_state() -> AppState {
    AppState::new(test_pool().await, test_config("http://127.0.0.1:3000")).unwrap()
}

/// Binds first so the submitted Origin matches the app's BASE_URL.
async fn spawn_app() -> (String, AppState) {
    spawn_app_with_key(None).await
}

async fn spawn_app_with_key(key: Option<&str>) -> (String, AppState) {
    let pool = test_pool().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let mut config = test_config(&base);
    config.admin_key = key.map(str::to_string);
    let state = AppState::new(pool, config).unwrap();
    let app = routes::router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (base, state)
}

async fn add_feed(pool: &SqlitePool, url: &str, status: &str) -> i64 {
    sqlx::query(
        "INSERT INTO feeds (url, site_url, domain, title, category, status)
         VALUES (?, 'https://example.com', 'example.com', 'Fixture Feed', 'tech', ?)",
    )
    .bind(url)
    .bind(status)
    .execute(pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn add_post(pool: &SqlitePool, feed_id: i64, guid: &str, title: &str, published_at: &str) {
    sqlx::query(
        "INSERT INTO posts (feed_id, guid, url, title, content_html, summary, published_at)
         VALUES (?, ?, 'https://example.com/post', ?, '<p>body</p>', 'summary', ?)",
    )
    .bind(feed_id)
    .bind(guid)
    .bind(title)
    .bind(published_at)
    .execute(pool)
    .await
    .unwrap();
}

async fn post_count(pool: &SqlitePool, feed_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ---------- M2: fetch worker ----------

#[test]
fn fixtures_parse() {
    let rss = feed_rs::parser::parse(RSS).unwrap();
    assert_eq!(rss.entries.len(), 3);
    let atom = feed_rs::parser::parse(ATOM).unwrap();
    assert_eq!(atom.entries.len(), 2);
    let youtube = feed_rs::parser::parse(YOUTUBE).unwrap();
    assert_eq!(youtube.entries[0].id, "yt:video:dQw4w9WgXcQ");
    assert!(feed_rs::parser::parse(MALFORMED).is_err());
}

#[tokio::test]
async fn ingest_dedups_and_sanitizes() {
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed.xml", "active").await;
    let feed = feed_rs::parser::parse(RSS).unwrap();

    let first = fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();
    let second = fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();
    assert_eq!(first, 3);
    assert_eq!(second, 0);
    assert_eq!(post_count(&state.pool, id).await, 3);

    let content: String = sqlx::query_scalar(
        "SELECT content_html FROM posts WHERE feed_id = ? AND guid = 'fixture-1'",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert!(content.contains("<strong>world</strong>"));
    for forbidden in ["<script", "<iframe", "<style", "<form"] {
        assert!(!content.contains(forbidden), "sanitizer kept {forbidden}");
    }

    let summary: String =
        sqlx::query_scalar("SELECT summary FROM posts WHERE feed_id = ? AND guid = 'fixture-1'")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(summary.chars().count() <= 300);
    assert!(summary.contains("short summary"));
}

#[tokio::test]
async fn ingest_skips_unchanged_entries() {
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed.xml", "active").await;
    let feed = feed_rs::parser::parse(RSS).unwrap();
    fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();

    let hash: String = sqlx::query_scalar(
        "SELECT content_hash FROM posts WHERE feed_id = ? AND guid = 'fixture-1'",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    // A second ingest must not overwrite the row when the feed input is equal.
    sqlx::query(
        "UPDATE posts SET content_html = '<p>kept</p>' WHERE feed_id = ? AND guid = 'fixture-1'",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .unwrap();

    let second = fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();
    assert_eq!(second, 0);
    let (content, stored_hash): (String, String) = sqlx::query_as(
        "SELECT content_html, content_hash FROM posts WHERE feed_id = ? AND guid = 'fixture-1'",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(content, "<p>kept</p>");
    assert_eq!(stored_hash, hash);
}

#[tokio::test]
async fn ingest_repairs_a_fallback_date() {
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed.xml", "active").await;
    add_post(
        &state.pool,
        id,
        "fixture-1",
        "First post",
        "2026-11-13 00:00:00",
    )
    .await;
    let feed = feed_rs::parser::parse(RSS).unwrap();

    fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();

    let published: String = sqlx::query_scalar(
        "SELECT published_at FROM posts WHERE feed_id = ? AND guid = 'fixture-1'",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(published, "2025-01-01 10:00:00");
}

#[tokio::test]
async fn ingest_fills_a_stub_by_url() {
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed.xml", "active").await;
    let url = "https://example.com/posts/hello";
    sqlx::query(
        "INSERT INTO posts (feed_id, guid, url, title, content_html, summary, thumb_url,
         published_at) VALUES (?, ?, ?, 'Hello world', '', '', '', '2024-01-01 00:00:00')",
    )
    .bind(id)
    .bind(url)
    .bind(url)
    .execute(&state.pool)
    .await
    .unwrap();

    let xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>T</title>
        <item><guid isPermaLink="false">?p=42</guid><link>{url}</link>
        <title>Hello world!</title>
        <description><![CDATA[<p>Real body</p><img src="https://example.com/x.jpg">]]></description>
        <pubDate>Wed, 01 Jan 2025 10:00:00 GMT</pubDate></item></channel></rss>"#
    );
    let feed = crate::worker::discover::parse_feed(xml.as_bytes()).unwrap();
    let new = fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();
    assert_eq!(new, 0);
    assert_eq!(post_count(&state.pool, id).await, 1);
    let row: (String, String, String) =
        sqlx::query_as("SELECT guid, title, thumb_url FROM posts WHERE feed_id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(row.0, url, "the stub guid must survive the refresh");
    assert_eq!(row.1, "Hello world!");
    assert_eq!(row.2, "https://example.com/x.jpg");
}

#[tokio::test]
async fn ingest_resolves_relative_links() {
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://pjonori.blog/rss.xml", "active").await;
    let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>T</title>
        <item><link>/posts/hello</link><title>Hello</title>
        <pubDate>Wed, 01 Jan 2025 10:00:00 GMT</pubDate></item></channel></rss>"#;
    let feed = crate::worker::discover::parse_feed(xml.as_bytes()).unwrap();
    fetch::ingest(&state.pool, id, &feed, "https://pjonori.blog")
        .await
        .unwrap();
    let url: String = sqlx::query_scalar("SELECT url FROM posts WHERE feed_id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(url, "https://pjonori.blog/posts/hello");
}

#[tokio::test]
async fn refresh_handles_304_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/feed.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/feed.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: Some("\"abc\"".into()),
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        category: "tech".into(),
        images_fetched_at: None,
        avatar_url: None,
        favicon_fetched_at: None,
    };
    fetch::refresh(&state, &due).await.unwrap();
    assert_eq!(post_count(&state.pool, id).await, 0);
    let (errors, fetched): (i64, Option<String>) =
        sqlx::query_as("SELECT error_count, last_fetched_at FROM feeds WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(errors, 0);
    assert!(fetched.is_some());
}

#[tokio::test]
async fn refresh_success_adapts_interval() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_raw(RSS, "application/rss+xml"),
        )
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/feed.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/feed.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: None,
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        category: "tech".into(),
        images_fetched_at: None,
        avatar_url: None,
        favicon_fetched_at: None,
    };
    fetch::refresh(&state, &due).await.unwrap();

    assert_eq!(post_count(&state.pool, id).await, 3);
    let (interval, errors, etag): (i64, i64, Option<String>) =
        sqlx::query_as("SELECT fetch_interval_secs, error_count, etag FROM feeds WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(interval, 1800, ">=3 new posts halves the interval");
    assert_eq!(errors, 0);
    assert_eq!(etag.as_deref(), Some("\"v1\""));
}

#[tokio::test]
async fn refresh_rediscovers_html_feed_with_existing_posts() {
    let server = MockServer::start().await;
    let page = r#"<html><head><title>My Blog</title></head><body>
        <a href="/posts/one">First post title</a>
        <a href="/posts/two">Second post title</a>
        <a href="/posts/three">Third post title</a>
    </body></html>"#;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .mount(&server)
        .await;
    let state = test_state().await;
    let url = format!("{}/", server.uri());
    let id = add_feed(&state.pool, &url, "active").await;
    add_post(
        &state.pool,
        id,
        "https://example.com/old",
        "Old post",
        "2025-01-01 00:00:00",
    )
    .await;
    let due = DueFeed {
        id,
        url: url.clone(),
        site_url: server.uri(),
        etag: Some("\"page-etag\"".into()),
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        category: "tech".into(),
        images_fetched_at: None,
        avatar_url: None,
        favicon_fetched_at: None,
    };
    fetch::refresh(&state, &due).await.unwrap();

    assert_eq!(post_count(&state.pool, id).await, 4);
    let etag: Option<String> = sqlx::query_scalar("SELECT etag FROM feeds WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert!(etag.is_none(), "synthetic feeds must not cache page ETags");
}

#[tokio::test]
async fn refresh_failure_backs_off() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/feed.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/feed.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: None,
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        category: "tech".into(),
        images_fetched_at: None,
        avatar_url: None,
        favicon_fetched_at: None,
    };
    fetch::refresh(&state, &due).await.unwrap();

    let (errors, last_error, delay): (i64, Option<String>, i64) = sqlx::query_as(
        "SELECT error_count, last_error, strftime('%s', next_fetch_at) - strftime('%s','now')
         FROM feeds WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(errors, 1);
    assert_eq!(last_error.as_deref(), Some("HTTP 500"));
    assert!(
        (7100..=7300).contains(&delay),
        "3600 * 2^1 backoff, got {delay}"
    );
}

#[tokio::test]
async fn refresh_rejects_oversized_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/big.xml"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![b'a'; ssrf::MAX_BODY + 1], "text/xml"),
        )
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/big.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/big.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: None,
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        category: "tech".into(),
        images_fetched_at: None,
        avatar_url: None,
        favicon_fetched_at: None,
    };
    fetch::refresh(&state, &due).await.unwrap();
    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM feeds WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(last_error.unwrap().contains("too large"));
}

// ---------- SSRF ----------

#[tokio::test]
async fn ssrf_blocks_private_and_local_targets() {
    for blocked in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.1.1",
        "0.0.0.0",
        "0.1.2.3",
        "100.64.0.1",
        "::",
        "::1",
        "fc00::1",
        "fe80::1",
    ] {
        let ip = blocked.parse().unwrap();
        assert!(ssrf::is_blocked(ip), "{blocked} should be blocked");
    }
    for allowed in ["93.184.216.34", "2606:4700::1111"] {
        let ip = allowed.parse().unwrap();
        assert!(!ssrf::is_blocked(ip), "{allowed} should be allowed");
    }
    assert!(
        ssrf::check_url(&"http://10.0.0.1/feed".parse().unwrap())
            .await
            .is_err()
    );
    assert!(
        ssrf::check_url(&"ftp://example.com/feed".parse().unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn resolver_rejects_blocked_addresses() {
    use reqwest::dns::Resolve;

    let name = "0.0.0.0".parse().unwrap();
    assert!(ssrf::SafeResolver.resolve(name).await.is_err());
}

// ---------- M4: timeline ----------

#[tokio::test]
async fn timeline_validates_and_filters() {
    let (base, state) = spawn_app().await;
    let feed_a = add_feed(&state.pool, "https://a.example/feed", "active").await;
    let feed_b = add_feed(&state.pool, "https://b.example/feed", "active").await;
    let rejected = add_feed(&state.pool, "https://c.example/feed", "rejected").await;
    for index in 0..55 {
        add_post(
            &state.pool,
            feed_a,
            &format!("a-{index}"),
            &format!("A post {index}"),
            &format!("2025-01-{:02} 10:00:00", index % 28 + 1),
        )
        .await;
    }
    add_post(&state.pool, feed_b, "b-1", "B post", "2025-02-01 10:00:00").await;
    add_post(
        &state.pool,
        rejected,
        "c-1",
        "Rejected post",
        "2025-02-01 10:00:00",
    )
    .await;

    let client = Client::new();
    let page_one = client
        .get(format!("{base}/api/timeline?feeds={feed_a}"))
        .send()
        .await
        .unwrap();
    assert_eq!(page_one.status(), 200);
    let body = page_one.text().await.unwrap();
    assert_eq!(body.matches("data-post-id=").count(), 50);
    assert!(body.contains("A post"));
    assert!(!body.contains("B post"));
    assert!(!body.contains("Rejected post"));

    let page_two = client
        .get(format!("{base}/api/timeline?feeds={feed_a}&page=2"))
        .send()
        .await
        .unwrap();
    let body = page_two.text().await.unwrap();
    assert_eq!(body.matches("data-post-id=").count(), 5);

    for bad in ["feeds=abc", "feeds=1,abc", "feeds="] {
        let response = client
            .get(format!("{base}/api/timeline?{bad}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{bad} should be rejected");
    }
    let many = vec!["1"; 51].join(",");
    let response = client
        .get(format!("{base}/api/timeline?feeds={many}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn timeline_rejects_injection() {
    let (base, state) = spawn_app().await;
    let feed = add_feed(&state.pool, "https://a.example/feed", "active").await;
    add_post(&state.pool, feed, "a-1", "A post", "2025-01-01 10:00:00").await;

    let client = Client::new();
    for bad in ["1;DROP%20TABLE%20posts", "1%20OR%201%3D1", "1)--"] {
        let response = client
            .get(format!("{base}/api/timeline?feeds={bad}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{bad} should be rejected");
    }
    assert_eq!(post_count(&state.pool, feed).await, 1);
}

#[tokio::test]
async fn directory_orders_by_newest_post() {
    let (base, state) = spawn_app().await;
    let older = add_feed(&state.pool, "https://a.example/feed", "active").await;
    let newer = add_feed(&state.pool, "https://b.example/feed", "active").await;
    for (id, title) in [(older, "Older feed"), (newer, "Newer feed")] {
        sqlx::query("UPDATE feeds SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&state.pool)
            .await
            .unwrap();
    }
    add_post(&state.pool, older, "a1", "A post", "2026-11-12 00:00:00").await;
    add_post(&state.pool, newer, "b1", "B post", "2026-11-01 00:00:00").await;

    let html = Client::new()
        .get(format!("{base}/feeds"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.find("Older feed") < html.find("Newer feed"));
}

#[tokio::test]
async fn directory_sorts_by_popular_and_falls_back_to_recent() {
    let (base, state) = spawn_app().await;
    let small = add_feed(&state.pool, "https://a.example/feed", "active").await;
    let big = add_feed(&state.pool, "https://b.example/feed", "active").await;
    for (id, title) in [(small, "Small feed"), (big, "Big feed")] {
        sqlx::query("UPDATE feeds SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&state.pool)
            .await
            .unwrap();
    }
    add_post(&state.pool, small, "s1", "S post", "2026-11-12 00:00:00").await;
    for guid in ["b1", "b2", "b3"] {
        add_post(&state.pool, big, guid, "B post", "2026-11-01 00:00:00").await;
    }

    let client = Client::new();
    let popular = client
        .get(format!("{base}/feeds?sort=popular"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(popular.find("Big feed") < popular.find("Small feed"));

    // Unknown sort values fall back to newest-post-first.
    let fallback = client
        .get(format!("{base}/feeds?sort=bogus"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(fallback.find("Small feed") < fallback.find("Big feed"));
}

#[tokio::test]
async fn directory_sort_oldest_uses_feed_created_at() {
    let (base, state) = spawn_app().await;
    let first = add_feed(&state.pool, "https://a.example/feed", "active").await;
    let second = add_feed(&state.pool, "https://b.example/feed", "active").await;
    for (id, title, created) in [
        (first, "First feed", "2026-01-01 00:00:00"),
        (second, "Second feed", "2026-06-01 00:00:00"),
    ] {
        sqlx::query("UPDATE feeds SET title = ?, created_at = ? WHERE id = ?")
            .bind(title)
            .bind(created)
            .bind(id)
            .execute(&state.pool)
            .await
            .unwrap();
    }
    // The older feed has the newer post; oldest sort ignores post recency.
    add_post(&state.pool, first, "a1", "A post", "2026-01-01 00:00:00").await;
    add_post(&state.pool, second, "b1", "B post", "2026-11-12 00:00:00").await;

    let html = Client::new()
        .get(format!("{base}/feeds?sort=oldest"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.find("First feed") < html.find("Second feed"));
}

#[tokio::test]
async fn new_static_pages_render_in_shell() {
    let (base, _state) = spawn_app().await;
    let client = Client::new();
    for path in ["/websites", "/links", "/skills"] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(response.text().await.unwrap().contains("side-nav"));
    }
}

#[tokio::test]
async fn directory_query_values_are_bound() {
    let (base, state) = spawn_app().await;
    add_feed(&state.pool, "https://one.example/feed", "active").await;
    let client = Client::new();

    let injected = client
        .get(format!("{base}/feeds?q=%27%20OR%201%3D1--"))
        .send()
        .await
        .unwrap();
    assert_eq!(injected.status(), 200);
    assert!(!injected.text().await.unwrap().contains("Fixture Feed"));

    // Invalid categories never reach SQL; they fall back to no category filter.
    let category = client
        .get(format!("{base}/feeds?category=%27%20OR%201%3D1--"))
        .send()
        .await
        .unwrap();
    assert_eq!(category.status(), 200);
    assert!(category.text().await.unwrap().contains("Fixture Feed"));
}

#[test]
fn sqlite_time_round_trips_as_space_format() {
    use chrono::{TimeZone, Utc};

    let time = Utc.with_ymd_and_hms(2026, 11, 12, 10, 30, 0).unwrap();
    assert_eq!(fetch::sqlite_time(time), "2026-11-12 10:30:00");

    let from_feed = chrono::DateTime::parse_from_rfc3339("2026-11-12T10:30:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(fetch::sqlite_time(from_feed), "2026-11-12 10:30:00");

    let parsed =
        chrono::NaiveDateTime::parse_from_str("2026-11-12 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
    assert_eq!(parsed.and_utc(), time);
}

#[tokio::test]
async fn no_cookies_are_set() {
    let (base, state) = spawn_app().await;
    add_feed(&state.pool, "https://one.example/feed", "active").await;
    let client = Client::new();
    for path in ["/", "/my", "/about"] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(
            response.headers().get("set-cookie").is_none(),
            "{path} set a cookie"
        );
    }
    let home = client
        .get(format!("{base}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(home.contains("Fixture Feed"));
}

#[tokio::test]
async fn cache_and_security_headers_are_scoped() {
    let (base, state) = spawn_app().await;
    let feed = add_feed(&state.pool, "https://a.example/feed", "active").await;
    let client = Client::new();

    let page = client.get(format!("{base}/")).send().await.unwrap();
    let headers = page.headers();
    assert_eq!(
        headers.get("cache-control").unwrap().to_str().unwrap(),
        "public, s-maxage=600, stale-while-revalidate=86400"
    );
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "strict-origin-when-cross-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
        ("x-frame-options", "SAMEORIGIN"),
    ] {
        assert_eq!(headers.get(name).unwrap().to_str().unwrap(), value);
    }

    // Health checks never get the shared-cache policy.
    let health = client.get(format!("{base}/healthz")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    assert!(health.headers().get("cache-control").is_none());

    // The timeline is a public GET, so htmx polls can share a short cache.
    let timeline = client
        .get(format!("{base}/api/timeline?feeds={feed}"))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline.status(), 200);
    assert_eq!(
        timeline
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap(),
        "public, s-maxage=60, stale-while-revalidate=300"
    );
}

// ---------- M5: submission ----------

async fn submit(client: &Client, base: &str, url: &str) -> reqwest::Response {
    client
        .post(format!("{base}/submit"))
        .header(ORIGIN, base)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(format!("url={url}&category=tech"))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn submit_html_alternate_inserts_and_fetches() {
    let feed_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RSS, "application/rss+xml"))
        .mount(&feed_server)
        .await;
    let page = format!(
        r#"<html><head><link rel="alternate" type="application/rss+xml" href="{}/feed.xml"></head><body>site</body></html>"#,
        feed_server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .mount(&feed_server)
        .await;

    let (base, state) = spawn_app().await;
    let client = Client::builder().redirect(Policy::none()).build().unwrap();
    let url = format!("{}/page", feed_server.uri());
    let response = submit(&client, &base, &url).await;
    assert_eq!(response.status(), 303);
    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with("/f/"));
    assert!(location.ends_with("?msg=added"));

    let (status, title): (String, String) =
        sqlx::query_as("SELECT status, title FROM feeds ORDER BY id DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(status, "active");
    assert_eq!(title, "Fixture Feed");
    let feed_id: i64 = sqlx::query_scalar("SELECT id FROM feeds ORDER BY id DESC LIMIT 1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(post_count(&state.pool, feed_id).await, 3);

    let duplicate = submit(&client, &base, &url).await;
    assert_eq!(duplicate.status(), 400);
    assert!(duplicate.text().await.unwrap().contains("already"));
}

#[tokio::test]
async fn submit_rejects_missing_origin() {
    let (base, _state) = spawn_app().await;
    let client = Client::new();
    let response = client
        .post(format!("{base}/submit"))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("url=http://10.0.0.1/feed&category=tech")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn submit_rate_limits_after_five() {
    let (base, _state) = spawn_app().await;
    let client = Client::new();
    for attempt in 1..=5 {
        let response = submit(&client, &base, "http://10.0.0.1/feed").await;
        assert_eq!(response.status(), 400, "attempt {attempt}");
    }
    let sixth = submit(&client, &base, "http://10.0.0.1/feed").await;
    assert_eq!(sixth.status(), 429);
}

#[tokio::test]
async fn submit_blocks_redirect_to_private() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://10.0.0.1/feed"))
        .mount(&server)
        .await;
    let (base, state) = spawn_app().await;
    let client = Client::new();
    let response = submit(&client, &base, &format!("{}/redirect", server.uri())).await;
    assert_eq!(response.status(), 400);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn admin_key_requires_six_mixed_chars() {
    use crate::config::valid_admin_key;
    assert!(valid_admin_key("ab12cd"));
    assert!(valid_admin_key("A1b2C3"));
    assert!(!valid_admin_key("abcdef"));
    assert!(!valid_admin_key("123456"));
    assert!(!valid_admin_key("abc12"));
    assert!(!valid_admin_key("abc1234"));
    assert!(!valid_admin_key("abc-12"));
}

#[tokio::test]
async fn admin_requires_basic_auth() {
    let (base, _state) = spawn_app_with_key(Some("ab12cd")).await;
    let client = Client::new();
    let denied = client.get(format!("{base}/admin")).send().await.unwrap();
    assert_eq!(denied.status(), 401);
    let ok = client
        .get(format!("{base}/admin"))
        .basic_auth("admin", Some("ab12cd"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
}

#[tokio::test]
async fn admin_is_disabled_without_key() {
    let (base, _state) = spawn_app().await;
    let response = Client::new()
        .get(format!("{base}/admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn admin_delete_removes_feed_and_posts() {
    let (base, state) = spawn_app_with_key(Some("ab12cd")).await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    add_post(&state.pool, id, "guid", "A post", "2025-01-01 00:00:00").await;
    let client = Client::builder().redirect(Policy::none()).build().unwrap();
    let response = client
        .post(format!("{base}/admin/feeds/{id}/delete"))
        .header(ORIGIN, &base)
        .basic_auth("admin", Some("ab12cd"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
    assert_eq!(post_count(&state.pool, id).await, 0);
    let feeds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(feeds, 0);
}

#[test]
fn youtube_channel_id_is_extracted() {
    let html =
        r#"{"channelId":"UCaaaaaaaaaaaaaaaaaaaaaa","externalId":"UCbbbbbbbbbbbbbbbbbbbbbb"}"#;
    assert_eq!(
        crate::worker::discover::extract_channel_id(html).as_deref(),
        Some("UCaaaaaaaaaaaaaaaaaaaaaa")
    );
    let html = r#"<link href="https://www.youtube.com/channel/UCcccccccccccccccccccccc">"#;
    assert_eq!(
        crate::worker::discover::extract_channel_id(html).as_deref(),
        Some("UCcccccccccccccccccccccc")
    );
}

#[tokio::test]
async fn seed_placeholder_keeps_real_url_and_error() {
    let state = test_state().await;
    crate::seed::insert_placeholder(
        &state,
        "https://unreachable.invalid",
        "Dead Feed",
        "tech",
        "dns failed",
    )
    .await
    .unwrap();
    crate::seed::insert_placeholder(
        &state,
        "https://unreachable.invalid",
        "Dead Feed",
        "tech",
        "dns failed",
    )
    .await
    .unwrap();

    let (url, error, status): (String, Option<String>, String) =
        sqlx::query_as("SELECT url, last_error, status FROM feeds")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(url, "https://unreachable.invalid/");
    assert_eq!(error.as_deref(), Some("dns failed"));
    assert_eq!(status, "active");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn sitemap_locs_reads_urlset_and_index() {
    let index = r#"<sitemapindex><sitemap><loc>https://a.example/post-sitemap.xml</loc></sitemap></sitemapindex>"#;
    assert_eq!(
        crate::worker::discover::sitemap_locs(index),
        vec!["https://a.example/post-sitemap.xml"]
    );
    let urlset = r#"<urlset><url><loc> https://a.example/one </loc></url></urlset>"#;
    assert_eq!(
        crate::worker::discover::sitemap_locs(urlset),
        vec!["https://a.example/one"]
    );
}

#[test]
fn html_feed_extracts_article_links() {
    let html = r#"<html><head><title>My Blog</title></head><body>
        <a href="/about">About me</a>
        <a href="/post/one">First post</a>
        <a href="/post/two">Second post</a>
        <a href="/post/three">Third post</a>
        <a href="https://other.example/post/x">Elsewhere</a>
    </body></html>"#;
    let base = url::Url::parse("https://example.com/").unwrap();
    let feed = crate::worker::discover::html_feed(html, &base).unwrap();
    assert_eq!(feed.entries.len(), 3);
    assert_eq!(feed.title.unwrap().content, "My Blog");
    assert!(
        feed.entries
            .iter()
            .any(|entry| entry.title.as_ref().unwrap().content == "Second post")
    );
}

#[test]
fn youtube_guid_extracts_embed_id() {
    let post = Post {
        id: 1,
        feed_id: 1,
        feed_title: "t".into(),
        feed_domain: "d".into(),
        guid: "yt:video:dQw4w9WgXcQ".into(),
        url: "u".into(),
        title: "t".into(),
        author: None,
        summary: String::new(),
        content_html: String::new(),
        thumb_url: None,
        published_label: "Jan 01, 2025".into(),
    };
    assert_eq!(post.youtube_id(), Some("dQw4w9WgXcQ"));
}

#[test]
fn post_cover_image_uses_body_image_then_youtube_thumbnail() {
    let mut post = Post {
        id: 1,
        feed_id: 1,
        feed_title: "t".into(),
        feed_domain: "d".into(),
        guid: "g".into(),
        url: "https://example.com/posts/one".into(),
        title: "t".into(),
        author: None,
        summary: String::new(),
        content_html: String::new(),
        thumb_url: None,
        published_label: "2025-01-01".into(),
    };
    assert!(post.is_stub());
    assert!(post.cover_image().is_none());

    post.content_html = r#"<p><img src="/hero.png" alt=""></p>"#.into();
    assert!(!post.is_stub());
    assert_eq!(
        post.cover_image().as_deref(),
        Some("https://example.com/hero.png")
    );

    post.guid = "yt:video:dQw4w9WgXcQ".into();
    assert_eq!(
        post.cover_image().as_deref(),
        Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
    );
}

#[test]
fn feed_parser_drops_empty_text_elements() {
    let xml = br#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/">
<title>t</title><id>f</id><updated>2024-01-01T00:00:00Z</updated>
<entry><id>e</id><title>e</title><updated>2024-01-01T00:00:00Z</updated>
<media:group><media:description></media:description></media:group></entry>
</feed>"#;
    assert!(feed_rs::parser::parse(xml.as_slice()).is_err());
    let feed = crate::worker::discover::parse_feed(xml).unwrap();
    assert_eq!(feed.entries.len(), 1);
}

#[test]
fn youtube_thumb_is_derived_from_guid() {
    let mut post = PostRow {
        id: 1,
        feed_id: 1,
        feed_title: "t".into(),
        feed_domain: "d".into(),
        guid: "yt:video:dQw4w9WgXcQ".into(),
        url: "u".into(),
        title: "t".into(),
        author: None,
        summary: String::new(),
        thumb_url: None,
        published_label: "2025-01-01".into(),
    };
    assert_eq!(
        post.thumb().as_deref(),
        Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg")
    );
    // A cached local poster wins over the derived remote URL.
    post.thumb_url = Some("/static/assets/posts/abc.jpg".into());
    assert_eq!(
        post.thumb().as_deref(),
        Some("/static/assets/posts/abc.jpg")
    );
    post.guid = "guid".into();
    post.thumb_url = None;
    assert!(post.thumb().is_none());
    // Link-only stubs store '' for "no body image"; render it as a placeholder.
    post.thumb_url = Some(String::new());
    assert_eq!(
        post.thumb_or_placeholder(),
        crate::models::PLACEHOLDER_IMAGE
    );
}

#[test]
fn channel_images_read_avatar_and_both_banner_shapes() {
    let legacy = r#"<script>var ytInitialData = {"metadata":{"channelMetadataRenderer":
    {"avatar":{"thumbnails":[
      {"url":"https://yt3.googleusercontent.com/a=s88","width":88},
      {"url":"https://yt3.googleusercontent.com/a=s900","width":900}]}}},
      "header":{"banner":{"thumbnails":[
      {"url":"https://yt3.googleusercontent.com/b=w1060","width":1060},
      {"url":"https://yt3.googleusercontent.com/b=w2276","width":2276}]}}};</script>"#;
    let images = crate::worker::discover::extract_channel_images(legacy);
    assert_eq!(
        images.avatar.as_deref(),
        Some("https://yt3.googleusercontent.com/a=s900")
    );
    assert_eq!(
        images.banner.as_deref(),
        Some("https://yt3.googleusercontent.com/b=w2276")
    );

    let current = r#"<script>var ytInitialData = {"header":{"banner":{"imageBannerViewModel":
    {"image":{"sources":[
      {"url":"https://yt3.googleusercontent.com/c=w1060","width":1060},
      {"url":"https://yt3.googleusercontent.com/c=w2276","width":2276}]}}}}};</script>"#;
    let images = crate::worker::discover::extract_channel_images(current);
    assert_eq!(
        images.banner.as_deref(),
        Some("https://yt3.googleusercontent.com/c=w2276")
    );
}

#[test]
fn channel_images_reject_foreign_hosts() {
    let html = r#"<script>var ytInitialData = {"metadata":{"channelMetadataRenderer":
    {"avatar":{"thumbnails":[{"url":"https://evil.example/a.jpg","width":900}]}}},
    "header":{"banner":{"thumbnails":[{"url":"https://evil.example/b.jpg","width":2276}]}}};</script>"#;
    let images = crate::worker::discover::extract_channel_images(html);
    assert!(images.avatar.is_none());
    assert!(images.banner.is_none());
}

#[tokio::test]
async fn directory_card_renders_channel_avatar() {
    let (base, state) = spawn_app().await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    let avatar = "https://yt3.googleusercontent.com/ytc/abc=s900-c-k-c0x00ffffff-no-rj";
    sqlx::query("UPDATE feeds SET category = 'youtube', avatar_url = ? WHERE id = ?")
        .bind(avatar)
        .bind(id)
        .execute(&state.pool)
        .await
        .unwrap();
    let html = Client::new()
        .get(format!("{base}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains(avatar));
    assert!(html.contains(r#"width="48" height="48""#));
}

#[tokio::test]
async fn feed_page_renders_youtube_thumbnail() {
    let (base, state) = spawn_app().await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    add_post(
        &state.pool,
        id,
        "yt:video:dQw4w9WgXcQ",
        "Video",
        "2025-01-01 00:00:00",
    )
    .await;
    let html = Client::new()
        .get(format!("{base}/f/{id}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg"));
    assert!(html.contains(r#"width="96" height="54""#));
}

#[tokio::test]
async fn feed_pages_paginate_and_clamp() {
    let (base, state) = spawn_app().await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    for index in 0..55 {
        add_post(
            &state.pool,
            id,
            &format!("p-{index}"),
            &format!("Post {index}"),
            &format!("2025-01-{:02} 10:00:00", index % 28 + 1),
        )
        .await;
    }
    async fn page(client: &Client, base: &str, id: i64, page: i64) -> String {
        client
            .get(format!("{base}/f/{id}?page={page}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }

    let client = Client::new();
    let first = page(&client, &base, id, 1).await;
    assert_eq!(first.matches("data-post-id=").count(), 50);
    assert!(first.contains(&format!("/f/{id}?page=2")));
    assert!(!first.contains(">Newer<"));

    let second = page(&client, &base, id, 2).await;
    assert_eq!(second.matches("data-post-id=").count(), 5);
    assert!(second.contains(&format!("/f/{id}?page=1")));
    assert!(!second.contains(">Older<"));

    // A page past the end shows the last page, not an empty list.
    let clamped = page(&client, &base, id, 99).await;
    assert_eq!(clamped.matches("data-post-id=").count(), 5);
    assert!(clamped.contains(&format!("/f/{id}?page=1")));
    assert_eq!(
        page(&client, &base, id, 0)
            .await
            .matches("data-post-id=")
            .count(),
        50
    );
}

#[tokio::test]
async fn feed_search_filters_posts_and_renders_a_fragment() {
    let (base, state) = spawn_app().await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    add_post(
        &state.pool,
        id,
        "a",
        "Rust ownership",
        "2025-01-01 00:00:00",
    )
    .await;
    add_post(
        &state.pool,
        id,
        "b",
        "Design systems",
        "2025-01-02 00:00:00",
    )
    .await;
    let client = Client::new();

    let html = client
        .get(format!("{base}/f/{id}?q=rust"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("Rust ownership"));
    assert!(!html.contains("Design systems"));
    assert!(html.contains(r#"value="rust""#));

    let fragment = client
        .get(format!("{base}/f/{id}/posts?q=design"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(fragment.contains(r#"id="posts""#));
    assert!(fragment.contains("Design systems"));
    assert!(!fragment.contains("Rust ownership"));
    assert!(!fragment.contains("Topbar"));

    let empty = client
        .get(format!("{base}/f/{id}?q=missing"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(empty.contains("No posts match"));
}

#[tokio::test]
async fn remote_images_are_cached_once_and_rewritten() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/thumb.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .set_body_bytes(b"\xff\xd8\xff\xe0not-a-real-jpeg".to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let dir = std::env::temp_dir().join(format!("dopefeed-assets-{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    let original = format!("{}/thumb.jpg", server.uri());
    sqlx::query("UPDATE posts SET thumb_url = ? WHERE feed_id = ?")
        .bind(&original)
        .bind(id)
        .execute(&state.pool)
        .await
        .unwrap();
    add_post(&state.pool, id, "guid", "A post", "2025-01-01 00:00:00").await;
    sqlx::query("UPDATE posts SET thumb_url = ? WHERE guid = 'guid'")
        .bind(&original)
        .execute(&state.pool)
        .await
        .unwrap();

    crate::assets::localize_post_thumbs(&state.pool, &state.http, &dir, id).await;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT thumb_url FROM posts WHERE guid = 'guid'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    let stored = stored.unwrap();
    assert!(stored.starts_with("/static/assets/posts/"), "{stored}");
    assert!(stored.ends_with(".jpg"));

    // Second pass must reuse the file instead of calling the source again.
    crate::assets::localize_post_thumbs(&state.pool, &state.http, &dir, id).await;
    let local = dir.join("posts").join(stored.rsplit('/').next().unwrap());
    assert!(tokio::fs::metadata(&local).await.is_ok());
}

#[test]
fn channel_videos_are_lifted_from_the_page() {
    let html = r#"<script>var ytInitialData = {"contents":[
      {"lockupViewModel":{"contentId":"dQw4w9WgXcQ","metadata":{"lockupMetadataViewModel":{
        "title":{"content":"A video"},
        "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[
          {"text":{"content":"115k"}},{"text":{"content":"3 days ago"}}]}]}}}}}},
      {"lockupViewModel":{"contentId":"dQw4w9WgXcQ","metadata":{"lockupMetadataViewModel":{
        "title":{"content":"Duplicate"}}}}}
    ]};</script>"#;
    let videos = crate::worker::discover::extract_channel_videos(html);
    assert_eq!(videos.len(), 1);
    assert_eq!(videos[0].id, "dQw4w9WgXcQ");
    assert_eq!(videos[0].title, "A video");
    let age = chrono::Utc::now() - videos[0].published.unwrap();
    assert!((age.num_hours() - 72).abs() <= 1);
}

#[test]
fn channel_videos_parse_short_relative_dates() {
    let html = r#"<script>var ytInitialData = {"contents":[
      {"lockupViewModel":{"contentId":"dQw4w9WgXcQ","metadata":{"lockupMetadataViewModel":{
        "title":{"content":"A year old video"},
        "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[
          {"text":{"content":"1 yr ago"}}]}]}}}}}},
      {"lockupViewModel":{"contentId":"9bZkp7q19f0","metadata":{"lockupMetadataViewModel":{
        "title":{"content":"An eleven month old video"},
        "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[
          {"text":{"content":"11 mo ago"}}]}]}}}}}}
    ]};</script>"#;
    let videos = crate::worker::discover::extract_channel_videos(html);
    assert_eq!(videos.len(), 2);
    let year = (chrono::Utc::now() - videos[0].published.unwrap()).num_days();
    assert!((year - 365).abs() <= 1, "{year} days");
    let months = (chrono::Utc::now() - videos[1].published.unwrap()).num_days();
    assert!((months - 330).abs() <= 1, "{months} days");
}

#[test]
fn og_image_prefers_the_social_preview_and_resolves_relative_urls() {
    let base = url::Url::parse("https://example.com/posts/one").unwrap();
    let html = r#"<meta property="og:image" content="/cover.png">
                  <meta name="twitter:image" content="https://cdn.example.com/t.png">"#;
    assert_eq!(
        crate::worker::discover::extract_og_image(html, &base).as_deref(),
        Some("https://example.com/cover.png")
    );
    assert_eq!(
        crate::worker::discover::extract_og_image("<p>no image</p>", &base),
        None
    );
}

#[tokio::test]
async fn body_image_fills_a_missing_thumb() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hero.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"\x89PNG\r\n\x1a\nnot-a-real-png".to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let dir = std::env::temp_dir().join(format!("dopefeed-body-thumb-{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed", "active").await;
    add_post(&state.pool, id, "guid", "A post", "2025-01-01 00:00:00").await;
    sqlx::query(
        "UPDATE posts SET content_html = ?, url = 'https://example.com/post', thumb_url = NULL
         WHERE guid = 'guid'",
    )
    .bind(format!(r#"<p><img src="{}/hero.png"></p>"#, server.uri()))
    .execute(&state.pool)
    .await
    .unwrap();

    crate::assets::localize_post_thumbs(&state.pool, &state.http, &dir, id).await;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT thumb_url FROM posts WHERE guid = 'guid'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(
        stored
            .as_deref()
            .is_some_and(|value| value.starts_with("/static/assets/posts/")),
        "{stored:?}"
    );
}

/// Blob hosts cached assets only; placeholders and hotlinked images stay put.
#[test]
fn asset_url_maps_only_cached_assets() {
    let base = Some("https://store.public.blob.vercel-storage.com");
    assert_eq!(
        assets::url_with("/static/assets/posts/abc.jpg", base),
        "https://store.public.blob.vercel-storage.com/posts/abc.jpg"
    );
    assert_eq!(
        assets::url_with("/static/assets/posts/abc.jpg", None),
        "/static/assets/posts/abc.jpg"
    );
    assert_eq!(
        assets::url_with("/static/no-image-dl.jpg", base),
        "/static/no-image-dl.jpg"
    );
    assert_eq!(
        assets::url_with("https://i.ytimg.com/vi/x/hq.jpg", base),
        "https://i.ytimg.com/vi/x/hq.jpg"
    );
}
