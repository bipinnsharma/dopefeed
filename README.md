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
| `READ_ONLY` | `false` | Reject submission and admin mutations; enabled in the Vercel image |
| `ADMIN_KEY` | unset (admin off) | At least 12 printable ASCII characters; Basic-auth password for `/admin` |

## Routes

- `/` directory (categories, search, 24/page)
- `/feeds` htmx grid partial
- `/f/{id}` feed detail, 50 posts/page (`?page=N`)
- `/read/{id}` reader (sanitized HTML)
- `/my` local timeline shell (fed by `/api/timeline`)
- `/submit` add a feed (validated, auto-approved)
- `/admin` add/delete feeds (HTTP Basic, password = `ADMIN_KEY`)
- `/about`, `/terms`, `/privacy`, `/healthz`

## Frontend

- `static/tokens.css` — design tokens (recent.design) verbatim
- `static/app.css` — component styles, tokens only
- `static/app.js` — subscriptions + read state (~45 lines)
- `static/dev-annotate.js` — design annotator, debug builds only (see below)
- `static/htmx.min.js` — vendored htmx 2.0.4
- `static/fonts/` — InterVariable (jsDelivr `@fontsource-variable/inter`) and
  Departure Mono (departuremono.com)

The directory, feed pages and reader work without JavaScript. Only
subscribing and `/my` require it.

Debug builds also load the design annotator. Click **Annotate** (bottom right),
click any element, describe the change, then **Copy report** and paste it into
the chat. Notes persist in `localStorage`. Release builds never include the
script (`cfg!(debug_assertions)` guards the tag in `base.html`).

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
python3 scripts/topfeed_sync.py --self-check
sh scripts/tests/test_snapshot_safety.sh
```

Fixtures in `fixtures/` cover RSS 2.0, Atom, YouTube and malformed XML;
integration tests use `wiremock`.

The browser regression uses Playwright Python (test-only; no application Node/npm
build) and a temporary SQLite database:

```sh
python3 -m pip install -r scripts/tests/requirements.txt
python3 -m playwright install chromium
cargo build --bin dopefeed
python3 scripts/tests/test_browser_flows.py
```

`sh scripts/check.sh` runs the full offline gates, including the browser flow.
The snapshot publisher runs this gate before refreshing or uploading anything.
Install the Playwright dependency and Chromium above before running snapshots.
GitHub Actions runs the same checks on pushes and pull requests.

## Maintenance commands

```sh
cargo run -- refresh          # force every active feed due now
cargo run -- refresh-feed 234 # refresh one feed by ID
python3 scripts/topfeed_sync.py --apply  # match post lists to topfeed.xyz
cargo run -- optimize-images  # resize/re-encode cached images once
cargo run -- verify-assets    # fail if the DB references a missing image
cargo run -- repair-body-urls # one-time bounded repair of stored article links/images
```

Run `repair-body-urls` on the local publishing database (or its backup) after
deploying this fix and before publishing a new snapshot. It scans all historical
non-empty bodies in batches of 500, is safe to rerun, and is never run against a
database automatically during normal startup.

`scripts/topfeed_sync.py` is the reference sync: for every local feed that maps
to a topfeed.xyz feed, topfeed's entries are the source of truth. Missing posts
are inserted as link stubs, titles and dates are refreshed, and local-only rows
are removed. Feed names and URLs are never touched, and a topfeed feed with no
entries or an incomplete/ambiguous reference response is skipped before writes.
Matching uses exact normalized URLs and only one-to-one exact-host fallback;
unrelated hosts with the same public-suffix ending are never joined. Posts are keyed by the exact entry URL, like topfeed: the
`http`/`https`, `#fragment`, tracking-query and trailing-slash variants it lists
are separate posts. When a feed spells a URL differently (scheme, trailing
slash, fragment, old permalink), the sync reuses the existing row instead of
replacing it, so a refresh cannot drop the body. API responses are cached in
`data/topfeed-cache` for an hour, so a `--verify` right after `--apply` is
nearly free. It is a dry run
unless `--apply` is passed. The next RSS
refresh fills a stub's body and poster. `scripts/snapshot.sh` runs refresh, the
sync, image optimization and asset verification, snapshots the DB with
`.backup`, uploads `data/dopefeed.db.xz` to the `db-latest` GitHub release, then
commits and pushes. Vercel builds that push, so no deploy happens unless there is
something new to publish.

Each upload is named with a timestamp and content hash and is validated before
`data/snapshot.txt` is atomically updated. Earlier release assets are retained so
older pointers and image/code sets remain usable.

The snapshot publisher refuses staged or modified work and unrelated untracked
files before it refreshes anything. Untracked generated files under `static/assets/`
are retained and published; only `data/snapshot.txt` and `static/assets/` are staged.

To publish twice a day, install the launchd job (`scripts/com.dopefeed.snapshot.plist`
runs it at 07:00 and 21:00). Replace `__REPO__` and `__HOME__` with absolute paths
first, then load it:

```sh
sed "s#__REPO__#$PWD#g; s#__HOME__#$HOME#g" scripts/com.dopefeed.snapshot.plist \
    > ~/Library/LaunchAgents/com.dopefeed.snapshot.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.dopefeed.snapshot.plist
```

For Vercel, `.github/workflows/snapshot.yml` runs the same publisher at 07:00 and
19:00 UTC. It restores the latest release database, refreshes it, then publishes
the new snapshot pointer; the resulting push triggers Vercel's production build.
It can also be started manually from the GitHub Actions tab.

macOS will not let launchd read a checkout under `~/Documents`; keep the repo
somewhere like `~/dopefeed`. Output goes to `~/Library/Logs/dopefeed-snapshot.log`.

## Docker

```sh
docker build -t dopefeed .
docker run -p 3000:3000 -v dopefeed-data:/app/data dopefeed
```

The image runs as a non-root user and stores SQLite in `/app/data`.

### Vercel

`Dockerfile.vercel` runs the same binary on Vercel's Container Runtime. It seeds
`/tmp/dopefeed.db` from `data/dopefeed.db.xz` because the container filesystem is
read-only elsewhere. The build downloads that snapshot from the `db-latest`
GitHub release; `data/snapshot.txt` holds its URL, and changing it busts the
Docker layer cache so a new snapshot is picked up.

```sh
vercel link --project dopefeed
```

Set `BASE_URL=https://dopefeed.vercel.app` and `PORT=3000` in the project settings.
`WORKER=0` disables the in-process worker there so scale-from-zero never starts a
fetch storm; the data comes from the snapshot. The project is linked to
`github.com/bipinnsharma/dopefeed` and builds every push to `master`, so
`static/assets/` is tracked in git (233 MB) rather than uploaded through the
deploy API, while the DB snapshot stays out of history as a release asset.
`scripts/snapshot.sh` is the way to publish: it uploads the snapshot, commits and
pushes. It needs `GITHUB_TOKEN` (repo scope) or a stored GitHub credential.
`scripts/smoke.sh` checks a deployment afterwards.
Images are already resized (post thumbs 320px, avatars 96px, covers 1200px) by
`optimize-images`. The SQLite file and anything the worker caches are per-instance
and lost when the function scales to zero; submissions are not persistent on Vercel.
