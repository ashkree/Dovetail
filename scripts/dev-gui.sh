#!/usr/bin/env bash
# Start the GUI (`tauri dev`) on its own state, so a test run never touches ~/.local/share/dovetail
# or ~/.config/dovetail. Every Dovetail directory goes under target/dovetail-dev/ (the runtime dir,
# which holds sockets, under $XDG_RUNTIME_DIR to stay short); the repo's role prompts are copied in
# on each start. Sessions run under the default Claude account (no profiles in this config).
# Gotcha: sessions' MCP bridge and hooks run the installed ~/.local/bin/dovetail, not this build;
# when the tools changed, that binary is stale until it is reinstalled.
#
#   scripts/dev-gui.sh            # start the window
#   scripts/dev-gui.sh --reset    # wipe target/dovetail-dev first
#
# Real sessions started from it still cost money: give them a scratch repo and tiny prompts
# (AGENTS.md "How to work" 7).
set -euo pipefail
cd "$(dirname "$0")/.."

dev="$PWD/target/dovetail-dev"
if [[ "${1:-}" == "--reset" ]]; then
    rm -rf "$dev"
fi

export DOVETAIL_DATA_DIR="$dev/data"
export DOVETAIL_CONFIG_DIR="$dev/config"
export DOVETAIL_STATE_DIR="$dev/state"
export DOVETAIL_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}/dovetail-dev"

mkdir -p "$DOVETAIL_DATA_DIR/projects" "$DOVETAIL_CONFIG_DIR/roles" "$DOVETAIL_CONFIG_DIR/projects" \
    "$DOVETAIL_STATE_DIR" "$DOVETAIL_RUNTIME_DIR"
cp config/roles/*.md "$DOVETAIL_CONFIG_DIR/roles/"

if [[ ! -d gui/node_modules ]]; then
    npm --prefix gui install
fi

echo "dev-gui: state under $dev"
cd gui
exec npx tauri dev
