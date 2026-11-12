# PROJECT: dopefeed — Rust feed reader + curated directory (no auth)

A public RSS/Atom feed directory and clean reader. Anyone can browse, filter,
search, read posts, subscribe locally (browser-only, no accounts), and submit
feeds (auto-approved after validation). A background worker fetches feeds.
Visual identity: the design system extracted from recent.design (Appendix A).

## 0. Agent operating rules
1. Implement M0→M6 in order. After each: `cargo fmt`, `cargo clippy -- -D warnings`,
   `cargo test` pass + milestone acceptance checks verified. Commit `M<n>: <summary>`.
2. No features beyond this spec. No accounts, sessions, cookies, OAuth, admin UI.
   The ONLY client state is localStorage (§10). Simplest choice wins; log decisions
   in `DECISIONS.md`.
3. Latest stable crates; commit `Cargo.lock`. No Node/npm. No JS beyond vendored
   htmx + `static/app.js` (§10, ≤150 lines vanilla JS).
4. No `unwrap()`/`expect()` in handlers or worker. All SQL via sqlx binds.
5. Works fully offline: SQLite + fixture tests. Live fetching only needs network.

## 1. Stack (fixed)
axum, tokio, tower-http (trace, compression, fs), sqlx (SQLite, WAL, macros,
migrate, chrono), askama (render to String → Html), feed-rs, reqwest (rustls-tls,
gzip; default-tls off), ammonia, scraper, url, sha2, serde, serde_json, chrono,
tracing, tracing-subscriber, anyhow, thiserror, dotenvy. Dev: wiremock.
Frontend: server-rendered HTML + htmx + `static/tokens.css` (Appendix A, verbatim)
+ `static/app.css` (component styles using ONLY tokens) + `static/app.js`.

## 2. Repo layout
```
Cargo.toml  migrations/  fixtures/  data/ (gitignored)
static/  tokens.css  app.css  app.js  htmx.min.js  fonts/
templates/  base.html directory.html _feed_grid.html _feed_card.html feed.html
            reader.html my.html _post_list.html submit.html about.html error.html
src/
  main.rs  config.rs  error.rs  db.rs  models.rs
  routes/  directory.rs  reader.rs  my.rs  submit.rs  static_pages.rs
  worker/  mod.rs  fetch.rs  discover.rs  ssrf.rs
```

## 3. Config (`.env` + `.env.example`)
```
BIND_ADDR=127.0.0.1:3000
BASE_URL=http://localhost:3000
DATABASE_URL=sqlite://data/dopefeed.db?mode=rwc
RUST_LOG=info
FETCH_CONCURRENCY=8
```

## 4. Schema (migration 0001, exact)
```sql
CREATE TABLE feeds (
  id INTEGER PRIMARY KEY,
  url TEXT NOT NULL UNIQUE,
  site_url TEXT NOT NULL,
  domain TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  category TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',   -- active | rejected
  etag TEXT, last_modified TEXT,
  last_fetched_at TEXT,
  next_fetch_at TEXT NOT NULL DEFAULT (datetime('now')),
  fetch_interval_secs INTEGER NOT NULL DEFAULT 3600,
  error_count INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE posts (
  id INTEGER PRIMARY KEY,
  feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  guid TEXT NOT NULL,
  url TEXT NOT NULL,
  title TEXT NOT NULL,
  author TEXT,
  content_html TEXT NOT NULL,              -- ammonia-sanitized at ingest
  summary TEXT NOT NULL DEFAULT '',
  published_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(feed_id, guid)
);
CREATE INDEX idx_posts_feed_pub ON posts(feed_id, published_at DESC);
CREATE INDEX idx_posts_pub ON posts(published_at DESC);
CREATE INDEX idx_feeds_next ON feeds(status, next_fetch_at);
CREATE INDEX idx_feeds_cat ON feeds(category);
```
Derived: `post_count` = COUNT(posts); `has_new_post` = latest published_at within
48h. NO reader counts, NO users/subscriptions/read tables (client-side only).

### 4a. Timestamp storage (pinned · correctness)
Every timestamp column (`published_at`, `last_fetched_at`, `next_fetch_at`,
`created_at`) stores UTC `YYYY-MM-DD HH:MM:SS`. All writes go through one helper;
never store a raw RFC3339/`to_rfc3339()` string. The worker compares
`next_fetch_at <= datetime('now')` lexicographically, and `'T' > ' '`, so a
T-format value would never be due.

```rust
pub fn sqlite_time(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}
```

Regression test: `sqlite_time_round_trips_as_space_format` (parsed RFC3339 →
space format → parses back to the same instant).

## 5. Categories (fixed)
`ai, ceo, coding, design, dev, lifestyle, product, startup, tech, writing, youtube`
Chips: ALL AI CEO CODING DESIGN DEV LIFESTYLE PRODUCT STARTUP TECH WRITING YOUTUBE.

## 6. Routes
| Method | Path | Behavior |
|---|---|---|
| GET | / | Directory: hero, chips, search, grid (24/page) |
| GET | /feeds | htmx partial `_feed_grid.html`; `category`, `q`, `page` |
| GET | /f/{id} | Feed detail: header, stats, latest 50 posts |
| GET | /read/{id} | Reader view (article carries `data-post-id`) |
| GET | /my | Local timeline shell; loads via /api/timeline (§10) |
| GET | /api/timeline | htmx `_post_list.html`; `feeds=1,2,3` (≤50 int ids, active feeds only; invalid → 400), `page` (50/page) |
| GET | /submit | Submission form |
| POST | /submit | Validate → discover → insert status=active → fetch now (§9) |
| GET | /about /terms /privacy | Static pages |
| GET | /healthz | 200, checks `SELECT 1` |
Unknown ids → 404 error page. POST /submit requires Origin/Referer host ==
BASE_URL host, else 403. No other mutations exist.

### 6a. Dynamic SQL (pinned · security)
No user input ever enters the SQL *shape*. Use `sqlx::QueryBuilder` and bind every
value (directory filters and the timeline `IN` list). `category` is matched
against the §5 enum before use; timeline ids are parsed to `i64` and rejected with
400 otherwise; `q` is a bound `LIKE` value. Never use `AssertSqlSafe` or
`format!` to splice values into SQL.

Regression tests: `directory_query_values_are_bound`,
`timeline_rejects_injection`.

## 7. UI — design system binding (recent.design tokens)
Load order in base.html: `tokens.css` → `app.css` → htmx → `app.js` (defer).
`app.css` may ONLY reference `var(--…)` tokens for color/spacing/radius/shadow/
motion/font values (raw px allowed only for layout: widths, grid gaps use tokens).
Light mode only: `color-scheme: light` (token set has no dark variant).

Element → token mapping (implement exactly):
- html/body: bg `--color-surface`, text `--color-ink`, `--font-sans`,
  `--font-size-16`, `--leading-base`, antialiased. Container max-width 1100px.
- Wordmark "dopefeed": Inter `--font-weight-medium`, `--font-size-18`,
  `--tracking-tight`, color `--color-ink`. Nav links `--color-ink-secondary`,
  hover `--color-ink`, transition `--duration-base` `--ease-standard`.
- Hero: h1 "Read what matters." `--font-size-48` (mobile `--font-size-28`),
  weight 500, `--leading-tight`, `--tracking-tight`; sub `--font-size-17`
  `--color-ink-secondary`. Accent word or trailing dot in `--color-accent`.
- Meta text (domain • category, "N posts", dates): `--font-mono`
  `--font-size-11`, `--color-ink-tertiary`. This is the signature recent.design look.
- Chips: `--radius-full`, border 1px `--color-border`, padding `--space-6`
  `--space-12`, `--font-size-13`. Hover bg `--color-fill-hover`. Active: bg
  `--color-ink`, text `--color-ink-inverse`, border transparent.
- Search input: `--radius-8`, border `--color-border`, padding `--space-8`
  `--space-12`; focus outline 2px `--color-focus`, offset 2px.
- Feed card: bg `--color-surface`, `--radius-12`, `--shadow-card`, padding
  `--space-16`; hover `--shadow-md` translateY(-1px), `--duration-base`
  `--ease-out-expo`. Title `--font-size-16` weight 500. Grid: 1col → 2 @640 →
  3 @1024 (breakpoints from tokens doc), gap `--space-16`.
- "new post" pill: `--font-mono` `--font-size-10`, color `--color-accent`,
  bg `color-mix(in oklch, var(--color-accent) 10%, transparent)`, `--radius-full`.
- Subscribe button [+]/[✓] (client-side, §10): 28px square, `--radius-8`,
  border `--color-border`; hover bg `--color-fill-hover`; subscribed state bg
  `--color-ink` text `--color-ink-inverse`, content ✓. `aria-pressed` required.
- Post list rows: padding-block `--space-12`, hairline bottom border
  `--color-border`; title `--font-size-16` weight 500 color `--color-ink`;
  summary `--font-size-13` `--color-ink-secondary`, 1 line clamp; date meta mono.
  Read state `.is-read`: opacity .55 (set by app.js).
- Reader: max-width 720px, `--font-size-17`, line-height 1.7; fluid h1
  `clamp(30px,5vw,44px)` weight 500, tight tracking; favicon crumb to the feed;
  byline mono meta. Cover = first `<img>` in the entry (else YouTube thumbnail,
  else a 180px favicon panel), `--radius-16` + `--shadow-card`; `og:image` and
  the per-feed favicon are set in the head. Posts with no stored body render a
  centered "summary only" card with the feed favicon, summary and a
  `Continue reading` button to the source. Links `--color-accent` underline,
  hover `--color-accent-strong`; code/pre `--font-mono` `--font-size-14`
  bg `--color-surface-subtle` `--radius-8`; blockquote border-left 2px
  `--color-border`; images max-width 100%, `--radius-12`. YouTube posts embed
  `https://www.youtube-nocookie.com/embed/{id}` 16:9 above content.
- Buttons (Submit form etc.): bg `--color-ink`, text `--color-ink-inverse`,
  `--radius-8`, padding `--space-8` `--space-16`, weight 500, hover opacity .9.
- Flash banner: bg `--color-warning-surface`, text `--color-warning-ink`,
  `--radius-8`. Errors use `--color-danger-surface`/`--color-danger-ink`.
- Empty states: `--color-ink-tertiary`, centered, `--space-48` block padding.
- Global: `:focus-visible { outline: 2px solid var(--color-focus); }` and
  `@media (prefers-reduced-motion: reduce) { * { transition: none !important;
  animation: none !important; } }`.
- Directory behavior: chips + search (keyup debounce 300ms) via htmx GET /feeds
  swapping `#feed-grid`; "Showing X of Y feeds" + "Load more" (append, hide at
  end); ordering `created_at DESC, id DESC`; empty search → "No feeds found."
- Static assets: `Cache-Control: public, max-age=86400` in release; `no-cache`
  in debug so CSS/JS edits show without hard-refreshing.

## 8. Fetch worker (in-process)
Tick 60s: select ≤50 active feeds with `next_fetch_at <= now`, oldest first.
Semaphore = FETCH_CONCURRENCY. Politeness: never 2 in-flight per domain; ≥2s
between same-domain fetches (in-memory map). reqwest: 90s timeout, UA
`dopefeed/1.0 (+{BASE_URL})`, conditional GET (stored ETag/Last-Modified), ≤5
redirects, 10MB body cap. 304 → success, skip parse. Parse feed-rs; per entry:
guid = id else link else sha256(title+link) [sha2]; skip if no title AND link;
published = published else updated else now; content = body else summary else "";
ammonia-sanitize (strip script/style/form/iframe); summary = 300 chars de-tagged.
Upsert ON CONFLICT(feed_id,guid) UPDATE title/content/summary/author (never
published_at); cap 500 entries/fetch. Success: store etag/last_modified,
error_count=0, next = now + interval; adaptive: ≥3 new → interval=max(900,i/2);
10 consecutive zero-new → interval=min(86400,i*2). Failure: error_count+=1,
last_error truncated, next = now + min(86400, i*2^min(error_count,6)). Update
feed.title from parsed feed only while post_count==0. Never panic the loop.
YouTube feeds: `https://www.youtube.com/feeds/videos.xml?channel_id=UC…`;
guid `yt:video:ID` → reader extracts ID for embed.

## 9. Submission (no review queue — auto-approve)
Fields: `url` (feed/site/YouTube channel), `category`, optional `title`.
1. SSRF (worker/ssrf.rs): http(s) only; resolve DNS; block loopback/private/
   link-local/unique-local (v4+v6); re-check every hop (manual redirects ≤5).
2. Parse as feed (≥1 entry) → use. Else HTML → `<link rel="alternate"
   type="application/rss+xml|atom+xml|feed+json">` candidates, first that
   validates. Else synthesize an Atom feed from the largest group of same-site
   article links, then from `/sitemap.xml` (or `/sitemap_index.xml`) as a last
   resort. Else YouTube channel URL → extract channelId
   (`"channelId":"UC…"` or canonical) → build feed URL. Else reject with
   friendly error.
3. Normalize URL; duplicate in any status → message, no insert.
4. Insert status=active (title override else feed title else domain; site_url =
   first entry origin else input origin), next_fetch_at=now. Redirect to
   /f/{id}?msg=added.
Rate limit: 5 submissions/hour/IP (in-memory). Body limit 64KB.

### 9a. YouTube discovery (pinned)
For any youtube.com URL fetch the channel page with a browser User-Agent and
`Cookie: CONSENT=YES+; SOCS=CAI`. Extract the channel id from the first match of
`"channelId":"UC…"`, `"externalId":"UC…"`, `"browseId":"UC…"`,
`/channel/(UC[\w-]{22})`, or `itemprop="identifier" content="UC…"`. Build
`https://www.youtube.com/feeds/videos.xml?channel_id={id}` and require ≥1 entry
before inserting; a 404/empty feed means the id is wrong. A `/channel/UC…` URL or
`?channel_id=UC…` is used directly. feed-rs rejects empty text elements (common in
YouTube feeds), so `parse_feed` strips exact empty element pairs and retries.

### 9b. SSRF hardening (pinned · security)
Validate every redirect hop and pin the resolved addresses so the connection uses
exactly what was checked (defeats DNS rebinding). `SafeResolver` implements
`reqwest::dns::Resolve`; it rejects the request when any resolved address is
blocked. Blocklist: loopback, private, link-local, unique-local, multicast, CGNAT,
`0.0.0.0/8`, and IPv6 `::` (unspecified); re-check the embedded IPv4 of
IPv4-mapped addresses.

Test-only loopback exception is the cargo feature `ssrf-test-exception` (plus
`cfg!(test)` for unit tests), never a release path. Release builds leave the
feature off. Regression tests: `ssrf_blocks_private_and_local_targets`,
`resolver_rejects_blocked_addresses`.

## 10. Local subscriptions & read state (`static/app.js`, no server state)
localStorage keys: `dopefeed.subs` = JSON array of feed ids;
`dopefeed.read` = JSON array of post ids (FIFO cap 5000).
- On every page: for each `[data-feed-id]` subscribe button, render ✓/pressed if
  subscribed. Click toggles the array + button state. No fetch calls.
- /my page: container `#timeline` with `hx-get="/api/timeline"`, `hx-trigger=
  "load"`; app.js injects `hx-vals='js:{feeds: …}'` from localStorage BEFORE htmx
  processes (or calls `htmx.ajax` directly). No subs → render empty state
  "Subscribe to feeds to build your timeline" + link to /.
- Read tracking: on /read, push `article[data-post-id]` id into read set. On any
  post list, add `.is-read` to rows whose `data-post-id` is in the set.
- "Export/Import" of subs: stretch only, not v1.
- Graceful degradation: directory/feed/reader fully work without JS; only
  subscribing and /my require JS (note this in README).

## 11. Seed (`cargo run -- seed`, idempotent)
The curated list lives in `src/seed.rs`: 177 titles/categories/URLs covering
design, coding, AI, startups and YouTube. `discover:` marks a site URL; the fetcher
auto-discovers its feed before insert.

### 11a. Seed placeholder (pinned · correctness)
When a seed URL cannot be resolved or fetched, insert a placeholder whose `url` is
the real site URL — never the literal `discover:` prefix. Set `last_error`, keep
`status='active'`, and let the worker retry discovery for 0-post rows with normal
backoff. Log dead seeds at `WARN`. Idempotent via the unique `url`.

Acceptance: every discover: YouTube row with a public channel resolves to a
verified `channel_id` feed and is inserted; after seeding,
`SELECT count(*) FROM feeds WHERE category='youtube'` ≥ 10.

## 12. Milestones & acceptance
M0 Skeleton: config/error/db/router//healthz/tracing/graceful shutdown/.env.example.
   ✅ cargo run serves /healthz 200; DB created; checks clean.
M1 Schema+seed+models. ✅ seed twice → no dupes; directory query returns counts.
M2 Worker + fixtures (rss2/atom/youtube/malformed) via wiremock.
   ✅ dedup on refetch; 304 skip; backoff math; 2MB cap; sanitizer strips script.
M3 Design system + public UI: Appendix A → static/tokens.css verbatim; app.css
   per §7; fonts: download InterVariable.woff2 (rsms/inter releases) +
   DepartureMono-Regular.woff2 (departuremono.com) into static/fonts/ and enable
   the @font-face block; if download impossible, leave commented (fallbacks OK)
   and note it. Routes /, /feeds, /f/{id}, /read/{id}, static pages.
   ✅ / links tokens.css; cards use --shadow-card; chips/filter/search/load-more
   work via htmx; meta text is mono; reader renders sanitized content; 404s work;
   grep app.css: no hex/rgb literals (tokens only).
M4 Local subs + /my: app.js per §10; /api/timeline.
   ✅ Tests: timeline rejects non-int/>50 ids (400), returns only requested
   feeds' posts, paginates; manual: subscribe → ✓ persists across reload → /my
   shows posts → open post → row dimmed on return. No cookies set anywhere.
M5 Submit auto-approve + SSRF + rate limit.
   ✅ wiremock HTML-with-alternate → feed active + fetched; duplicate rejected;
   SSRF blocks 127.0.0.1/10.x/169.254.x/::1/redirect-to-private; 6th submit in
   an hour → 429; POST without Origin → 403.
M6 Hardening + deploy: compression, static cache headers, request tracing,
   multi-stage Dockerfile (non-root, VOLUME /app/data), README (setup/seed/
   fonts/limits), DECISIONS.md. ✅ docker build ok; container /healthz 200.

### 12a. Review regression tests (required)
- `sqlite_time_round_trips_as_space_format` — parsed RFC3339 → space format →
  parsed back to the same instant.
- `directory_query_values_are_bound` — `q=' OR 1=1--` matches nothing; invalid
  `category` never reaches SQL.
- `timeline_rejects_injection` — `feeds=1;DROP TABLE posts` → 400; posts intact.
- `ssrf_blocks_private_and_local_targets` — includes `0.0.0.0/8` and `::`.
- `resolver_rejects_blocked_addresses` — the validating resolver rejects a
  blocked address before connect.
- `seed_placeholder` — placeholder URL has no `discover:` prefix and
  `last_error` is set (covered by live seed + worker retry path).

## 13. Definition of Done
- [ ] Fresh clone: `cp .env.example .env && cargo run -- seed && cargo run` →
      populated directory within 2 min, styled per §7.
- [ ] Subscribe 3 feeds → reload persists → /my timeline → read post → dimmed.
- [ ] Chips/search/load-more work without full reloads; reduced-motion honored.
- [ ] Submit real blog URL → appears in directory immediately.
- [ ] No auth anywhere: no cookies, no users table, no login routes.
- [ ] fmt/clippy/test clean; no unwrap in handlers/worker; only sanitized HTML
      rendered |safe; app.css contains no raw color literals.
- [ ] No timestamp column contains a `T` separator.
- [ ] No user input reaches a SQL shape string; dynamic queries use QueryBuilder.
- [ ] SSRF connects only to validated addresses; `0.0.0.0/8` and `::` blocked.
- [ ] `cargo test` passes; `cargo test --features ssrf-test-exception` passes.

## 14. Non-goals (v1)
Accounts/auth/profiles/admin/review queue, server-side read state, email digest,
OPML, comments, dark mode (token set is light-only), multi-instance workers.

## Appendix A — static/tokens.css (save verbatim; from recent.design extraction)
```css
/* dopefeed design tokens — extracted from https://recent.design (light only).
   Hex = sRGB fallback; oklch = source of truth (wins in modern browsers). */
:root {
  --color-surface:#ffffff; --color-surface:oklch(100% 0 0);
  --color-surface-subtle:#f5f5f5; --color-surface-subtle:oklch(97% 0 0);
  --color-surface-muted:#f0f0f0; --color-surface-muted:oklch(95.5% 0 0);
  --color-border:#ebebeb; --color-border:oklch(94% 0 0);
  --color-ink:#0b0b0b; --color-ink:oklch(15% 0 0);
  --color-ink-secondary:#0000007c; --color-ink-secondary:oklch(0% 0 0/.486);
  --color-ink-tertiary:#00000044; --color-ink-tertiary:oklch(0% 0 0/.267);
  --color-ink-inverse:#fdfdfd; --color-ink-inverse:oklch(99.5% 0 0);
  --color-fill-hover:#00000010; --color-fill-hover:oklch(0% 0 0/.063);
  --color-fill-active:#00000017; --color-fill-active:oklch(0% 0 0/.09);
  --color-fill-strong:#0000001f; --color-fill-strong:oklch(0% 0 0/.122);
  --color-overlay:#0b0b0b66; --color-overlay:oklch(15% 0 0/.4);
  --color-accent:#ff4000; --color-accent:oklch(68% .27 48);
  --color-accent-strong:#f62b00; --color-accent-strong:oklch(62% .26 48);
  --color-highlight:#f900a3; --color-highlight:oklch(64% .28 350);
  --color-highlight-strong:#e00090; --color-highlight-strong:oklch(58% .269 350);
  --color-focus:#2287ff; --color-focus:oklch(66% .246 260);
  --color-success:#00c500; --color-success:oklch(70% .27 145);
  --color-success-ink:#003700; --color-success-ink:oklch(28% .13 145);
  --color-warning:#fccc00; --color-warning:oklch(86% .21 95);
  --color-warning-ink:#573f00; --color-warning-ink:oklch(38% .12 95);
  --color-warning-surface:#fefcf4; --color-warning-surface:oklch(99% .011 95);
  --color-danger:#ff0005; --color-danger:oklch(62% .29 25);
  --color-danger-ink:#5c0000; --color-danger-ink:oklch(28% .14 25);
  --color-danger-surface:#ffe2dd; --color-danger-surface:oklch(95% .052 25);
  --color-danger-border:#ed0000; --color-danger-border:oklch(56% .278 25);

  --font-sans:"Inter",ui-sans-serif,system-ui,sans-serif;
  --font-mono:"Departure Mono",ui-monospace,monospace;
  --font-size-9:9px; --font-size-10:10px; --font-size-11:11px; --font-size-12:12px;
  --font-size-13:13px; --font-size-14:14px; --font-size-16:16px; --font-size-17:17px;
  --font-size-18:18px; --font-size-20:20px; --font-size-28:28px; --font-size-32:32px;
  --font-size-48:48px;
  --font-weight-regular:400; --font-weight-medium:500;
  --leading-none:1; --leading-tight:1.2; --leading-snug:1.35; --leading-base:1.4;
  --leading-relaxed:1.5;
  --tracking-tight:-0.0175em;

  --space-2:2px; --space-4:4px; --space-6:6px; --space-8:8px; --space-10:10px;
  --space-12:12px; --space-16:16px; --space-20:20px; --space-24:24px;
  --space-32:32px; --space-40:40px; --space-48:48px;

  --radius-2:2px; --radius-3:3px; --radius-4:4px; --radius-8:8px; --radius-10:10px;
  --radius-12:12px; --radius-16:16px; --radius-18:18px; --radius-full:9999px;
  --radius-circle:50%;

  --shadow-card:0 0 0 .5px oklch(0% 0 0/.08),0 1px 1px -1px oklch(0% 0 0/.2),0 1px 2px oklch(0% 0 0/.08);
  --shadow-sm:0 1px 2px oklch(0% 0 0/.06),0 0 0 1px oklch(0% 0 0/.05);
  --shadow-md:0 1px 3px oklch(0% 0 0/.06),0 4px 12px oklch(0% 0 0/.05),0 0 0 1px oklch(0% 0 0/.04);
  --shadow-lg:0 2px 5px oklch(0% 0 0/.06),0 10px 24px oklch(0% 0 0/.07),0 0 0 1px oklch(0% 0 0/.03);
  --shadow-hairline-inset:inset 0 0 0 1px oklch(0% 0 0/.09);
  --shadow-ring:0 0 0 2px oklch(0% 0 0/.063);

  --duration-fast:120ms; --duration-base:180ms; --duration-medium:200ms;
  --duration-slow:300ms; --duration-slower:500ms;
  --ease-out-expo:cubic-bezier(0.16,1,0.3,1);
  --ease-standard:cubic-bezier(0.4,0,0.2,1);
  --ease-emphasized:cubic-bezier(0.2,0,0,1);
}
/* Breakpoints (can't be CSS vars in media queries): sm 640 · md 768 · lg 1024 · xl 1280 */
/* Enable after placing fonts in static/fonts/ (M3):
@font-face{font-family:"Inter";src:url("/static/fonts/InterVariable.woff2") format("woff2");
  font-weight:100 900;font-style:normal;font-display:swap;}
@font-face{font-family:"Departure Mono";src:url("/static/fonts/DepartureMono-Regular.woff2") format("woff2");
  font-weight:400;font-style:normal;font-display:swap;} */
```