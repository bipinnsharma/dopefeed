# Decisions

Short log of choices that were not fully pinned down by `agent.md`.

## Runtime / dependencies

- **sqlx 0.9 `AssertSqlSafe`** — 0.9 only accepts `&'static str` for query
  strings. Two queries need a dynamic `WHERE`/`IN` shape (directory filters,
  timeline ids). Only the SQL *shape* is dynamic; every value is bound. Those
  strings are wrapped in `AssertSqlSafe` so the audit is explicit.
- **SQL date format** — `published_at`, `last_fetched_at` and `next_fetch_at`
  are stored as UTC `YYYY-MM-DD HH:MM:SS` so SQLite `datetime()`/`strftime()`
  comparisons work lexicographically.
- **Zero-new-streak in memory** — the fixed schema has no column for
  "consecutive zero-new fetches", so the adaptive interval streak lives in an
  in-process map. It resets on restart; acceptable for a single-instance
  worker.
- **Seed fallback** — if a seed URL cannot be fetched (offline or dead), a
  placeholder feed row is inserted so search/directory UI is usable
  immediately. The worker retries it with normal backoff. Idempotent via the
  unique `url`.
- **Feed title updates** — follow the spec: only while `post_count == 0`, so a
  later feed-title change never overrides a user-provided title.

## UI

- **Load more** — `/feeds` accepts an internal `more=1` parameter (only used
  by htmx) to return just the next grid fragment, which avoids duplicating the
  count line or nesting grid wrappers. The normal `category`/`q`/`page` API is
  unchanged.
- **htmx version** — 2.0.4 vendored from unpkg.
- **Fonts** — `InterVariable.woff2` comes from the Fontsource variable package
  on jsDelivr (the rsms GitHub release download was blocked); Departure Mono
  from departuremono.com. `@font-face` is enabled in `tokens.css`.
- **No dark mode** — token set is light-only; `color-scheme: light`.
- **Static caching** — `/static/*` gets `Cache-Control: public, max-age=86400`
  via middleware; HTML is uncached.

## Security

- **SSRF** — every hop is resolved and checked; private, loopback, link-local,
  unique-local, multicast, CGNAT and IPv4-mapped addresses are blocked.
- **Test loopback exception** — `cfg!(test)` allows a loopback resolution so
  `wiremock` can exercise discovery/submission in unit tests. Release builds
  never take this branch.
- **Origin check** — POST `/submit` compares the Origin (or Referer) host and
  port with `BASE_URL`. Requests without either header are rejected.
- **Sanitization** — ammonia's default policy is applied on ingest; scripts,
  styles, forms and iframes are removed by default. Covered by a fixture test.

## Deliberate omissions

No accounts, sessions, cookies, admin UI, OPML/export, comments, dark mode or
multi-instance coordination. Nothing beyond `agent.md` v1.
