#!/usr/bin/env bash
# Install (or upgrade) the stable Dovetail into ~/.local/bin: `dovetail` (the core, the frozen TUI,
# and the hooks and MCP bridge every session runs) and `dovetail-gui` (the window, ADR 0023: a
# release build with the web files inside, no dev server).
#
#   scripts/install.sh            # check, then install both
#   scripts/install.sh --no-gui   # only `dovetail`
#   scripts/install.sh --check    # only check whether Dovetail is running (exit 1 if so)
#
# Refuses while anything runs the installed binary: the app itself, and the hooks and MCP
# bridges its sessions start, so replacing it under them is how upgrades break running work
# (plan R5). Quit Dovetail, run this, open it again; sessions resume.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

target="$HOME/.local/bin/dovetail"
gui_target="$HOME/.local/bin/dovetail-gui"
check_only=0
gui=1
[[ "${1:-}" == "--check" ]] && check_only=1
[[ "${1:-}" == "--no-gui" ]] && gui=0

# 1. Any process running the installed binary (app, `dovetail mcp`, `dovetail hook`). This works
#    from inside a Dovetail slice too, where the runtime-dir variables point at a dev directory.
running=()
for bin in "$target" "$gui_target"; do
  [[ -e "$bin" ]] || continue
  real="$(readlink -f "$bin")"
  for exe in /proc/[0-9]*/exe; do
    link="$(readlink "$exe" 2>/dev/null)" || continue
    # A binary replaced while running shows as "<path> (deleted)".
    if [[ "$link" == "$real" || "$link" == "$real (deleted)" ]]; then
      pid="${exe#/proc/}"; pid="${pid%/exe}"
      running+=("pid $pid")
    fi
  done
done

# 2. A listening app socket in the live runtime dir: XDG_RUNTIME_DIR, else the temp dir Rust's
#    std::env::temp_dir() uses (TMPDIR, else /tmp). Never DOVETAIL_RUNTIME_DIR: inside a slice
#    that is the dev build's directory.
runtime="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/dovetail"
if [[ -d "$runtime" ]]; then
  have_ss=0
  listening=""
  if command -v ss >/dev/null; then
    have_ss=1
    # Captured first: `ss | grep -q` under pipefail can fail with SIGPIPE when grep exits early.
    listening="$(ss -xlH 2>/dev/null || true)"
  fi
  for sock in "$runtime"/*.sock; do
    [[ -S "$sock" ]] || continue
    # Without ss every socket file counts; with it, only one something listens on (a crash can
    # leave a stale file behind).
    if [[ $have_ss -eq 0 || "$listening" == *"$sock"* ]]; then
      running+=("$sock")
    fi
  done
fi

if [[ ${#running[@]} -gt 0 ]]; then
  echo "close Dovetail first: it is running (${running[*]})" >&2
  exit 1
fi
if [[ $check_only -eq 1 ]]; then
  echo "Dovetail is not running"
  exit 0
fi

cargo install --locked --path crates/dovetail --root "$HOME/.local"
echo "installed $target"

if [[ $gui -eq 1 ]]; then
  # `tauri build` runs `npm run build` and embeds gui/dist in the binary; --no-bundle skips the
  # .deb/AppImage packaging, which a local install does not need.
  [[ -d gui/node_modules ]] || npm --prefix gui ci
  (cd gui && npx tauri build --no-bundle)
  install -m 755 target/release/dovetail-gui "$gui_target"
  echo "installed $gui_target (start it with: dovetail-gui)"
fi
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo "note: $HOME/.local/bin is not on PATH; add it so \`dovetail\` runs the installed build" ;;
esac
