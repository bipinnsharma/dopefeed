# Decisions

Short log of choices that were not fully pinned down by `agent.md`.

## Runtime / dependencies

- **Dynamic SQL** — two queries need a dynamic `WHERE`/`IN` shape (directory
  filters, timeline ids). They use `sqlx::QueryBuilder`; only fixed SQL fragments
  are concatenated and every value is bound. `category` is checked against the
  fixed enum before use. No `AssertSqlSafe` remains.
- **SQL date format** — `published_at`, `last_fetched_at` and `next_fetch_at`
  are stored as UTC `YYYY-MM-DD HH:MM:SS` via `fetch::sqlite_time`, so SQLite
  `datetime()`/`strftime()` comparisons work lexicographically. Never store raw
  RFC3339 (`T` sorts above the space in `datetime('now')` and would stop fetching).
- **Zero-new-streak in memory** — the fixed schema has no column for
  "consecutive zero-new fetches", so the adaptive interval streak lives in an
  in-process map. It resets on restart; acceptable for a single-instance
  worker.
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
  in release and `no-cache` in debug, so asset edits show immediately; HTML is
  uncached.

## Seed

- **Seed fallback** — if a seed URL cannot be fetched (offline or dead), a
  placeholder feed row is inserted using the real site URL (never the
  `discover:` prefix) with `last_error` set. The worker re-runs discovery for
  0-post rows before backing off. Idempotent via the unique `url`.

## Security

- **SSRF** — every hop is resolved and checked, and a validating
  `reqwest::dns::Resolve` (`SafeResolver`) makes reqwest connect only to the
  addresses that passed the check, closing the DNS-rebinding/TOCTOU gap. Private,
  loopback, link-local, unique-local, multicast, CGNAT, `0.0.0.0/8` and IPv6 `::`
  addresses are blocked.
- **Test loopback exception** — the cargo feature `ssrf-test-exception` (plus
  `cfg!(test)` for unit tests) allows a loopback resolution so `wiremock` can
  exercise discovery/submission. Release builds never enable the feature.
- **Origin check** — POST `/submit` compares the Origin (or Referer) host and
  port with `BASE_URL`. Requests without either header are rejected.
- **Sanitization** — ammonia's default policy is applied on ingest; scripts,
  styles, forms and iframes are removed by default. Covered by a fixture test.

## Deliberate omissions

No accounts, sessions, cookies, admin UI, OPML/export, comments, dark mode or
multi-instance coordination. Nothing beyond `agent.md` v1.
