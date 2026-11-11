#!/usr/bin/env bash
# Long-session memory (M16): streams ~10,000 turns through the UI on synthetic events and prints
# resident memory at the end and at its peak. Proposed target: under 100 MB. Release build, since
# debug builds are several times larger.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --release -p dovetail-tui --test rss -- --ignored --nocapture 2>&1 | grep -E 'turns=|test result|panicked'
