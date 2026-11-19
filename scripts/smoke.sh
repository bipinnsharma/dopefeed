#!/bin/sh
# Post-deploy smoke test: pages render and images come from Blob and load.
# Usage: scripts/smoke.sh [base-url]
set -eu
base="${1:-https://dopefeed.vercel.app}"
pages=$(mktemp)
trap 'rm -f "$pages"' EXIT
fail=0

check_page() {
    if ! curl -fsS --max-time 20 "$base$1" >> "$pages"; then
        echo "FAIL page $1" >&2
        fail=1
    fi
}

check_page /
feed=$(grep -o 'href="/f/[0-9]*"' "$pages" | head -1 | tr -d 'href="' || true)
post=$(grep -o 'href="/read/[0-9]*"' "$pages" | head -1 | tr -d 'href="' || true)
if [ -n "$feed" ]; then check_page "$feed"; else echo "FAIL no feed link" >&2; fail=1; fi
if [ -n "$post" ]; then check_page "$post"; else echo "FAIL no read link" >&2; fail=1; fi

grep -q 'blob\.vercel-storage\.com' "$pages" ||
    { echo "FAIL rendered html has no blob urls" >&2; fail=1; }

imgs=$(grep -o 'https://[0-9a-z]*\.public\.blob\.vercel-storage\.com/[^" ]*' "$pages" | sort -u || true)
if [ -z "$imgs" ]; then
    echo "FAIL no blob images to sample" >&2
    exit 1
fi
bad=$(printf '%s\n' "$imgs" | awk 'BEGIN{srand()} {print rand(), $0}' | sort -n |
    cut -d' ' -f2- | head -20 | while read -r url; do
        code=$(curl -sI -o /dev/null -w '%{http_code}' --max-time 20 "$url" || echo 000)
        [ "$code" = 200 ] || echo "$code $url"
    done)
if [ -n "$bad" ]; then
    echo "FAIL sampled images:" >&2
    printf '%s\n' "$bad" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "smoke test FAILED: $base" >&2
    exit 1
fi
echo "smoke test ok: $base"
