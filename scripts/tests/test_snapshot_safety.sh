#!/bin/sh
set -eu

tmp=$(mktemp -d "${TMPDIR:-/tmp}/dopefeed-snapshot-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
real_git=$(command -v git)
real_python=$(command -v python3)
script_dir=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)

setup_repo() {
    repo=$1
    mkdir -p "$repo"/scripts "$repo"/static/assets/posts "$repo"/data "$repo"/src "$repo"/bin
    cp "$script_dir/snapshot.sh" "$repo/scripts/snapshot.sh"
    cp "$script_dir/../.gitignore" "$repo/.gitignore"
    cat > "$repo/scripts/check.sh" <<'STUB'
#!/bin/sh
[ -z "${SNAPSHOT_CHECK_LOG:-}" ] || echo called >> "$SNAPSHOT_CHECK_LOG"
STUB
    cd "$repo"
    "$real_git" init -q -b main
    "$real_git" config user.email test@example.invalid
    "$real_git" config user.name Test
    printf 'source\n' > src/main.rs
    printf 'asset\n' > static/assets/posts/kept.jpg
    printf 'https://github.com/bipinnsharma/dopefeed/releases/download/db-latest/previous.db.xz\n' > data/snapshot.txt
    "$real_git" add .gitignore scripts/snapshot.sh scripts/check.sh src/main.rs static data/snapshot.txt
    "$real_git" commit -qm baseline
}

# Staged and untracked changes must fail before any snapshot command runs.
for kind in staged untracked; do
    repo="$tmp/$kind"
    setup_repo "$repo"
    if [ "$kind" = staged ]; then
        printf 'user edit\n' >> src/main.rs
        "$real_git" add src/main.rs
    else
        printf 'user file\n' > unrelated.txt
    fi
    log="$tmp/$kind.log"
    if (cd "$repo" && TMPDIR="$tmp" SNAPSHOT_CHECK_LOG="$log" sh scripts/snapshot.sh); then
        echo "expected $kind checkout to be rejected" >&2
        exit 1
    fi
    [ ! -e "$log" ] || { echo "snapshot command ran before rejecting $kind checkout" >&2; exit 1; }
done

# A clean run stages only generated publication outputs, never unrelated work.
repo="$tmp/clean"
setup_repo "$repo"
printf worker-output > "$repo/static/assets/posts/preexisting-worker.jpg"
mockbin="$tmp/mockbin"
mkdir -p "$mockbin"
cat > "$mockbin/git" <<STUB
#!/bin/sh
if [ "\$1" = commit ]; then
    "$real_git" diff --cached --name-only > "\$SNAPSHOT_STAGED"
    exit 88
fi
exec "$real_git" "\$@"
STUB
cat > "$mockbin/cargo" <<'STUB'
#!/bin/sh
case "$*" in
    *optimize-images*) mkdir -p static/assets/posts; printf generated > static/assets/posts/generated.jpg; printf unrelated > unrelated.txt ;;
esac
STUB
cat > "$mockbin/python3" <<STUB
#!/bin/sh
if [ "\$1" = scripts/topfeed_sync.py ]; then exit 0; fi
exec "$real_python" "\$@"
STUB
cat > "$mockbin/sqlite3" <<'STUB'
#!/bin/sh
touch data/dopefeed.snapshot.db
STUB
cat > "$mockbin/xz" <<'STUB'
#!/bin/sh
touch "$3.xz"
rm "$3"
STUB
cat > "$mockbin/curl" <<'STUB'
#!/bin/sh
echo "$*" >> "$SNAPSHOT_CURL_LOG"
case "$*" in
    *releases/tags/db-latest*) printf '{"id":123,"assets":[{"id":456,"name":"previous.db.xz"}]}\n' ;;
    *uploads.github.com*)
        [ "${SNAPSHOT_FAIL_UPLOAD:-}" = 1 ] && exit 22
        name=$(printf '%s' "$*" | sed -n 's/.*?name=//p')
        size=$(wc -c < data/dopefeed.db.xz | tr -d '[:space:]')
        if [ "${SNAPSHOT_BAD_RESPONSE:-}" = 1 ]; then
            printf '{"name":"%s","size":0,"state":"uploaded","browser_download_url":"bad"}\n' "$name"
            exit 0
        fi
        printf '{"name":"%s","size":%s,"state":"uploaded","browser_download_url":"https://github.com/bipinnsharma/dopefeed/releases/download/db-latest/%s"}\n' "$name" "$size" "$name"
        ;;
esac
STUB
chmod +x "$mockbin/"*

# A failed upload must leave the previous pointer untouched.
repo="$tmp/failed-upload"
setup_repo "$repo"
old_pointer=$(cat "$repo/data/snapshot.txt")
curl_log="$tmp/failed-curl.log"
if (cd "$repo" && PATH="$mockbin:$PATH" TMPDIR="$tmp" GITHUB_TOKEN=test SNAPSHOT_CURL_LOG="$curl_log" SNAPSHOT_FAIL_UPLOAD=1 sh scripts/snapshot.sh); then
    echo "expected mocked upload failure" >&2
    exit 1
fi
[ "$(cat "$repo/data/snapshot.txt")" = "$old_pointer" ] || { echo "failed upload changed the pointer" >&2; exit 1; }

repo="$tmp/invalid-response"
setup_repo "$repo"
old_pointer=$(cat "$repo/data/snapshot.txt")
if (cd "$repo" && PATH="$mockbin:$PATH" TMPDIR="$tmp" GITHUB_TOKEN=test SNAPSHOT_CURL_LOG="$curl_log" SNAPSHOT_BAD_RESPONSE=1 sh scripts/snapshot.sh); then
    echo "expected invalid upload metadata to be rejected" >&2
    exit 1
fi
[ "$(cat "$repo/data/snapshot.txt")" = "$old_pointer" ] || { echo "invalid upload response changed the pointer" >&2; exit 1; }

repo="$tmp/clean"
staged="$tmp/staged-files"
curl_log="$tmp/success-curl.log"
old_pointer=$(cat "$repo/data/snapshot.txt")
if (cd "$repo" && PATH="$mockbin:$PATH" TMPDIR="$tmp" GITHUB_TOKEN=test SNAPSHOT_STAGED="$staged" SNAPSHOT_CURL_LOG="$curl_log" sh scripts/snapshot.sh); then
    echo "expected mocked git commit to stop the publication" >&2
    exit 1
fi
grep -Fx 'data/snapshot.txt' "$staged" >/dev/null
grep -Fx 'static/assets/posts/generated.jpg' "$staged" >/dev/null
grep -Fx 'static/assets/posts/preexisting-worker.jpg' "$staged" >/dev/null
[ "$(cat "$repo/data/snapshot.txt")" != "$old_pointer" ] || { echo "successful upload did not update the pointer" >&2; exit 1; }
if grep -- '-X DELETE' "$curl_log" >/dev/null; then
    echo "publisher deleted an earlier snapshot asset" >&2
    exit 1
fi
if grep -E '(^|/)src/|unrelated' "$staged" >/dev/null; then
    echo "unrelated work was staged" >&2
    exit 1
fi
echo "snapshot safety checks passed"
