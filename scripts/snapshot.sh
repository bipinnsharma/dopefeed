#!/bin/sh
# Publish a fresh snapshot: refresh content, re-encode new images, then push
# the DB and assets. Vercel builds each push from GitHub, so nothing goes
# through the deploy file-upload API. Run weekly or on demand.
set -eu
cd "$(dirname "$0")/.."

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
asset_id=$(printf '%s' "$release" | python3 -c 'import json,sys;print(next((a["id"] for a in json.load(sys.stdin)["assets"] if a["name"]=="dopefeed.db.xz"),""))')
if [ -n "$asset_id" ]; then
    curl -fsS -X DELETE -H "Authorization: token $token" "$api/releases/assets/$asset_id"
fi
curl -fsS -X POST -H "Authorization: token $token" -H "Content-Type: application/octet-stream" \
    --data-binary @data/dopefeed.db.xz \
    "https://uploads.github.com/repos/bipinnsharma/dopefeed/releases/$release_id/assets?name=dopefeed.db.xz" > /dev/null
printf 'https://github.com/bipinnsharma/dopefeed/releases/download/db-latest/dopefeed.db.xz?v=%s\n' "$(date +%s)" > data/snapshot.txt

git add -A
if git diff --cached --quiet; then
    echo "nothing changed; no deploy"
    exit 0
fi
git commit -m "Snapshot $(date +%Y-%m-%d)"
git push origin "$(git branch --show-current)"
