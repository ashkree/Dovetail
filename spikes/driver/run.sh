#!/usr/bin/env bash
# S1 driver spike: set up the scratch repo and config, build, and run one check.
# Real Claude Code sessions: tiny prompts, scratch repo only.
# Usage: run.sh <raw-check>|sdkrs|claw     (raw checks: a b c d e g bg todo pt)
#   g needs: run.sh g <session-id> "<prompt>" [log-name]
#   pt takes: noflag | sub
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
base=/tmp/dovetail-spikes/driver
export CARGO_TARGET_DIR=$base/target

mkdir -p "$base/repo" "$base/cfg" "$base/outside"
if [ ! -d "$base/repo/.git" ]; then
  git -C "$base/repo" init -q -b main
  echo "# scratch" > "$base/repo/README.md"
  git -C "$base/repo" add . && git -C "$base/repo" -c user.name=spike -c user.email=spike@example.invalid commit -qm init
fi
sed "s#SPIKE_DIR#$here#" "$here/cfg/settings.example.json" > "$base/cfg/settings.json"
cp "$here/cfg/append.md" "$base/cfg/append.md"
repo_root=$(cd "$here/../.." && pwd)
printf '{"mcpServers":{"spike":{"type":"stdio","command":"python3","args":["%s/spikes/mcp/server.py"],"env":{"SPIKE_LOG":"%s/mcp.log"}}}}\n' \
  "$repo_root" "$base" > "$base/cfg/mcp.json"

case "${1:-}" in
  sdkrs) (cd "$here/sdkrs" && cargo build -q --release) && cd "$base" && "$CARGO_TARGET_DIR/release/spike-sdkrs" ;;
  claw)  (cd "$here/claw" && cargo build -q --release) && cd "$base" && "$CARGO_TARGET_DIR/release/spike-claw" ;;
  "")    echo "usage: run.sh <a|b|c|d|e|g|bg|todo|pt|sdkrs|claw> [args]"; exit 2 ;;
  *)     (cd "$here/raw" && cargo build -q --release) && cd "$base" && "$CARGO_TARGET_DIR/release/spike-raw" "$@" ;;
esac
