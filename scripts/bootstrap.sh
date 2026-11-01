#!/usr/bin/env bash
# Prepare this machine for running Dovetail, including on Dovetail's own repo.
# Safe to re-run: it never overwrites files you have already edited.
#
#   scripts/bootstrap.sh            # set up dirs, install role prompts, check tools
#   scripts/bootstrap.sh --force    # also overwrite installed role prompts with the repo's
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
force=0
[[ "${1:-}" == "--force" ]] && force=1

data="${XDG_DATA_HOME:-$HOME/.local/share}/dovetail"
config="${XDG_CONFIG_HOME:-$HOME/.config}/dovetail"
state="${XDG_STATE_HOME:-$HOME/.local/state}/dovetail"

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$*"; }

echo "Directories"
mkdir -p "$data/projects" "$config/roles" "$config/projects" "$state"
ok "$data"
ok "$config"
ok "$state"

echo "Role prompts"
for f in "$repo_root"/config/roles/*.md; do
  dest="$config/roles/$(basename "$f")"
  if [[ -e "$dest" && $force -eq 0 ]]; then
    if cmp -s "$f" "$dest"; then ok "$(basename "$f") (up to date)"
    else warn "$(basename "$f") differs from the repo's copy; kept yours (--force to replace)"; fi
  else
    cp "$f" "$dest"; ok "$(basename "$f") installed"
  fi
done

echo "Project recipe for Dovetail itself"
recipe="$config/projects/dovetail.toml"
if [[ -e "$recipe" ]]; then
  ok "kept existing $recipe"
else
  sed "s|^repo = .*|repo = \"$repo_root\"|" "$repo_root/config/projects/dovetail.toml" > "$recipe"
  ok "installed $recipe (repo = $repo_root)"
fi

echo "Tools"
need_version="2.1.224"
if command -v claude >/dev/null; then
  have="$(claude --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"
  if [[ -n "$have" && "$(printf '%s\n%s\n' "$need_version" "$have" | sort -V | head -1)" == "$need_version" ]]; then
    ok "claude $have (≥ $need_version for cross-session messaging)"
  else
    fail "claude ${have:-unknown}: need ≥ $need_version"
  fi
else
  fail "claude not found on PATH"
fi
command -v cargo >/dev/null && ok "$(cargo --version)" || fail "cargo not found (install rustup)"
command -v git   >/dev/null && ok "$(git --version)"   || fail "git not found"
command -v nvim  >/dev/null && ok "nvim (notes and worktree hand-off)" || warn "nvim not found (needed later for notes hand-off)"
if [[ "${TERM:-}" == *kitty* || -n "${KITTY_WINDOW_ID:-}" ]]; then
  ok "kitty detected (map image canvas available)"
else
  warn "not running in kitty: maps fall back to the outline view"
fi

echo "Memory caps (systemd-run --user)"
if command -v systemd-run >/dev/null; then
  if systemd-run --user --scope --quiet -p MemoryMax=64M true 2>/dev/null; then
    ok "per-session cgroup caps work without root"
  else
    fail "systemd-run --user --scope -p MemoryMax=… failed: caps will be unavailable (open spike)"
  fi
else
  fail "systemd-run not found: caps will be unavailable"
fi

echo "Messaging blockers"
blocked=0
for v in CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC DISABLE_TELEMETRY DO_NOT_TRACK DISABLE_GROWTHBOOK; do
  if [[ -n "${!v:-}" ]]; then warn "$v is set: it can silently disable cross-session messaging"; blocked=1; fi
done
[[ $blocked -eq 0 ]] && ok "none set"

echo
echo "Done. Nothing was written into any repository."
