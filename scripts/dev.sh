#!/usr/bin/env bash
# Run the TUI on its own state, so a test run never touches ~/.local/share/dovetail or
# ~/.config/dovetail. Every Dovetail directory goes under target/dovetail-dev/ (the runtime dir,
# which holds sockets, under $XDG_RUNTIME_DIR to stay short); the repo's role prompts are copied in
# on each start. Sessions run under the default Claude account (no profiles in this config).
# Gotcha: sessions' MCP bridge and hooks run the installed ~/.local/bin/dovetail, not this build;
# when the tools changed, that binary is stale until it is reinstalled.
#
#   scripts/dev.sh                # run the TUI (extra arguments go to `dovetail`)
#   scripts/dev.sh --reset        # wipe target/dovetail-dev first, then run
#
# Real sessions started from it still cost money: give them a scratch repo and tiny prompts
# (AGENTS.md "How to work" 7).
set -euo pipefail
cd "$(dirname "$0")/.."

dev="$PWD/target/dovetail-dev"
args=()
for arg in "$@"; do
    case "$arg" in
        --reset) rm -rf "$dev" ;;
        *) args+=("$arg") ;;
    esac
done

export DOVETAIL_DATA_DIR="$dev/data"
export DOVETAIL_CONFIG_DIR="$dev/config"
export DOVETAIL_STATE_DIR="$dev/state"
export DOVETAIL_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}/dovetail-dev"

mkdir -p "$DOVETAIL_DATA_DIR/projects" "$DOVETAIL_CONFIG_DIR/roles" "$DOVETAIL_CONFIG_DIR/projects" \
    "$DOVETAIL_STATE_DIR" "$DOVETAIL_RUNTIME_DIR"
cp config/roles/*.md "$DOVETAIL_CONFIG_DIR/roles/"

echo "dev: state under $dev" >&2
exec cargo run -p dovetail -- "${args[@]}"
