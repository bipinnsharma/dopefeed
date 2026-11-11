# dopefeed

A public RSS/Atom/YouTube feed directory and clean reader. No accounts, no
cookies, no tracking. Subscriptions and read state live in `localStorage`.

Server-rendered HTML + htmx, Rust (axum + sqlx/SQLite), with feed fetching in a
background worker.

## Quick start

```sh
cp .env.example .env
cargo run -- seed     # optional: populate the directory from the seed list
cargo run
```

Then open <http://localhost:3000>. A fresh database is created and migrated
automatically; feeds are fetched as the worker ticks (every 60s).

`cargo run -- seed` is idempotent. It validates each seed URL through the same
discovery path as submissions; if the network is unavailable it inserts a
placeholder row so the directory is still populated, and the worker keeps
retrying.

## Requirements

- Rust stable (edition 2024)
- SQLite is bundled through sqlx; nothing to install
- `cmake` on Linux for the rustls/aws-lc build

## Configuration (`.env`)

| Variable | Default | Meaning |
|---|---|---|
| `BIND_ADDR` | `127.0.0.1:3000` | Listen address |
| `BASE_URL` | `http://localhost:3000` | Public origin; used for the fetch User-Agent and Origin checks |
| `DATABASE_URL` | `sqlite://data/dopefeed.db?mode=rwc` | SQLite URL |
| `RUST_LOG` | `info` | Tracing filter |
| `FETCH_CONCURRENCY` | `8` | Parallel feed fetches |

## Routes

- `/` directory (categories, search, 24/page)
- `/feeds` htmx grid partial
- `/f/{id}` feed detail, latest 50 posts
- `/read/{id}` reader (sanitized HTML)
- `/my` local timeline shell (fed by `/api/timeline`)
- `/submit` add a feed (validated, auto-approved)
- `/about`, `/terms`, `/privacy`, `/healthz`

## Frontend

- `static/tokens.css` — design tokens (recent.design) verbatim
- `static/app.css` — component styles, tokens only
- `static/app.js` — subscriptions + read state (~45 lines)
- `static/htmx.min.js` — vendored htmx 2.0.4
- `static/fonts/` — InterVariable (jsDelivr `@fontsource-variable/inter`) and
  Departure Mono (departuremono.com)

The directory, feed pages and reader work without JavaScript. Only
subscribing and `/my` require it.

## Limits

- POST `/submit` body: 64KB
- Submissions: 5 per hour per IP (in-memory)
- Fetch: 20s timeout, 2MB body cap, 5 redirects, conditional GET, one request
  per domain at a time with 2s spacing
- Posts kept per feed: 500 per fetch; reader shows latest 50
- Sanitized HTML: ammonia on ingest; raw content is never rendered

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

Fixtures in `fixtures/` cover RSS 2.0, Atom, YouTube and malformed XML;
integration tests use `wiremock`.

## Docker

```sh
docker build -t dopefeed .
docker run -p 3000:3000 -v dopefeed-data:/app/data dopefeed
```

The image runs as a non-root user and stores SQLite in `/app/data`.
