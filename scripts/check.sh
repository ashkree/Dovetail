#!/usr/bin/env bash
# The wave gate (AGENTS.md "How to work" 3), plus mechanical checks for rules agents keep missing.
# Run before reporting a wave or a fix done. Exits non-zero on the first failure.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# AGENTS.md "Code conventions": typed (thiserror) errors in libraries; String errors only in the binary.
libs=(crates/dovetail-api crates/dovetail-core crates/dovetail-driver crates/dovetail-mock crates/dovetail-tui)
if hits=$(grep -rnE 'Result<[^;{]*,[[:space:]]*String>' "${libs[@]/%//src}"); then
    echo "check.sh: library code returns String errors; use a thiserror type:" >&2
    echo "$hits" >&2
    exit 1
fi

echo "check.sh: all checks passed"
