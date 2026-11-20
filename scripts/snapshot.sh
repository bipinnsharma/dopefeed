#!/bin/sh
# Publish a fresh snapshot: refresh content, re-encode new images, then push
# the DB and assets. Vercel builds each push from GitHub, so nothing goes
# through the deploy file-upload API. launchd runs it at 07:00 and 21:00.
set -eu
cd "$(dirname "$0")/.."

# Never mix a snapshot publication with unrelated staged, modified, or untracked work.
repo_root=$(git rev-parse --show-toplevel) || {
    echo "snapshot must run inside a git checkout" >&2
    exit 1
}
if [ -n "$(git rev-parse --show-prefix)" ]; then
    echo "snapshot must run from the checkout root" >&2
    exit 1
fi
if ! branch=$(git symbolic-ref --quiet --short HEAD); then
    echo "snapshot requires a named branch (not detached HEAD)" >&2
    exit 1
fi
if [ -n "$(git status --porcelain --untracked-files=all -- . ':(exclude)static/assets')" ] \
    || ! git diff --quiet HEAD -- static/assets; then
    echo "snapshot requires a clean checkout except for untracked generated static/assets; preserve or commit local work first" >&2
    exit 1
fi

# launchd runs this twice a day; never let two publishes overlap.
lock="${TMPDIR:-/tmp}/dopefeed-snapshot.lock"
if ! mkdir "$lock" 2>/dev/null; then
    if kill -0 "$(cat "$lock/pid" 2>/dev/null)" 2>/dev/null; then
        echo "snapshot already running; exiting" >&2
        exit 0
    fi
    rm -rf "$lock"
    mkdir "$lock"
fi
echo $$ > "$lock/pid"
snapshot_tmp=
trap '[ -z "$snapshot_tmp" ] || rm -f "$snapshot_tmp"; rm -rf "$lock"' EXIT

sh scripts/check.sh

cargo run --release -- refresh
python3 scripts/topfeed_sync.py --apply
cargo run --release -- optimize-images
cargo run --release -- verify-assets

# .backup reads through the WAL, so a running dev server cannot leave posts out
# of the snapshot the way a busy wal_checkpoint does.
sqlite3 data/dopefeed.db ".backup data/dopefeed.snapshot.db"
xz -9 -f data/dopefeed.snapshot.db
mv -f data/dopefeed.snapshot.db.xz data/dopefeed.db.xz

# The DB is 68 MB uncompressed, so it lives in the db-latest release rather
# than git history; the Docker build downloads it from there.
token=${GITHUB_TOKEN:-$(printf 'protocol=https\nhost=github.com\n' | git credential-osxkeychain get | sed -n 's/^password=//p')}
if [ -z "$token" ]; then
    echo "set GITHUB_TOKEN (repo scope) or store a GitHub credential" >&2
    exit 1
fi
api=https://api.github.com/repos/bipinnsharma/dopefeed
release=$(curl -fsS -H "Authorization: token $token" "$api/releases/tags/db-latest")
release_id=$(printf '%s' "$release" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
asset_hash=$(shasum -a 256 data/dopefeed.db.xz | cut -c1-16)
asset_name="dopefeed-$(date -u +%Y%m%dT%H%M%SZ)-$$-$asset_hash.db.xz"
asset_size=$(wc -c < data/dopefeed.db.xz | tr -d '[:space:]')
upload=$(curl -fsS -X POST -H "Authorization: token $token" -H "Content-Type: application/octet-stream" \
    --data-binary @data/dopefeed.db.xz \
    "https://uploads.github.com/repos/bipinnsharma/dopefeed/releases/$release_id/assets?name=$asset_name")
pointer=$(printf '%s' "$upload" | python3 -c '
import json, sys
asset = json.load(sys.stdin)
name, size = sys.argv[1], int(sys.argv[2])
url = f"https://github.com/bipinnsharma/dopefeed/releases/download/db-latest/{name}"
if asset.get("name") != name or asset.get("size") != size or asset.get("state") != "uploaded" or asset.get("browser_download_url") != url:
    raise SystemExit("uploaded snapshot asset failed validation")
print(url)
' "$asset_name" "$asset_size")
snapshot_tmp=$(mktemp data/snapshot.txt.XXXXXX)
printf '%s\n' "$pointer" > "$snapshot_tmp"
mv -f "$snapshot_tmp" data/snapshot.txt
snapshot_tmp=

git add -- data/snapshot.txt static/assets
if git diff --cached --quiet; then
    echo "nothing changed; no deploy"
    exit 0
fi
git commit -m "Snapshot $(date +%Y-%m-%d)"
git push origin "$(git branch --show-current)"
