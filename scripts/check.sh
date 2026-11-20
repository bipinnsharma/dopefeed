#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --features ssrf-test-exception
python3 scripts/topfeed_sync.py --self-check
sh -n scripts/snapshot.sh
sh scripts/tests/test_snapshot_safety.sh
cargo build --bin dopefeed
python3 scripts/tests/test_browser_flows.py
