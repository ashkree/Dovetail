#!/usr/bin/env bash
# Install (or upgrade) the stable Dovetail into ~/.local/bin, the binary you self-host with.
#
#   scripts/install.sh
#
# Refuses while any Dovetail app is running: its sessions' hooks and MCP bridge run the
# installed binary, so replacing it under them is how upgrades break running work (plan R5).
# Quit Dovetail, run this, open it again; sessions resume.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

runtime="${DOVETAIL_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/dovetail}"

# A running app listens on <runtime>/<project>.sock (and new-<pid>.sock before registration).
running=()
if [[ -d "$runtime" ]]; then
  for sock in "$runtime"/*.sock; do
    [[ -S "$sock" ]] || continue
    if command -v ss >/dev/null; then
      # Only a socket something listens on counts; a crash can leave a stale file behind.
      ss -xlH 2>/dev/null | grep -qF -- "$sock" && running+=("$sock")
    else
      running+=("$sock")
    fi
  done
fi
if [[ ${#running[@]} -gt 0 ]]; then
  echo "close Dovetail first: an app is running (${running[*]})" >&2
  exit 1
fi

cargo install --locked --path crates/dovetail --root "$HOME/.local"
echo "installed $HOME/.local/bin/dovetail"
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo "note: $HOME/.local/bin is not on PATH; add it so \`dovetail\` runs the installed build" ;;
esac
