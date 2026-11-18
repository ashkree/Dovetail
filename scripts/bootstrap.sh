#!/usr/bin/env bash
# Prepare this machine for running Dovetail, including on Dovetail's own repo.
# Safe to re-run: it never overwrites files you have already edited.
#
#   scripts/bootstrap.sh            # set up dirs, install role prompts, check tools
#   scripts/bootstrap.sh --force    # also overwrite installed role prompts and the recipe
#                                   # (needed after a prompt change, e.g. the tool rename of ADR 0016)
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
  # ADR 0016 renamed the tools (`ask_user` is `core_ask_user`, ...). The old names still answer
  # for one release, with a deprecation note; a prompt that still uses them needs --force.
  if grep -qE '`(ask_user|report_status|log_decision|create_task|start_slice|add_project|draw_map)`' "$dest"; then
    warn "$(basename "$f") still names the old tools (ADR 0016); run scripts/bootstrap.sh --force to install the renamed ones"
  fi
done

echo "Project recipe for Dovetail itself"
# The project id is the slug of the repo folder's name (lower-case, runs of other characters become
# one '-'), as dovetail_core::projects::slug does. If another repo already took the id, Dovetail
# numbers this one (e.g. dovetail-2); rename the installed file to match then.
project_id="$(basename "$repo_root" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//')"
[[ -n "$project_id" ]] || project_id="project"
recipe="$config/projects/$project_id.toml"
if [[ -e "$recipe" && $force -eq 0 ]]; then
  if cmp -s "$repo_root/config/projects/dovetail.toml" "$recipe"; then ok "$recipe (up to date)"
  else warn "$recipe differs from the repo's copy; kept yours (--force to replace)"; fi
else
  cp "$repo_root/config/projects/dovetail.toml" "$recipe"
  ok "installed $recipe (project id: $project_id)"
fi
# R1 (ADR 0015): Dovetail's research goes to docs/research/ only when the installed recipe says so.
if grep -qE '^\s*research_dir\s*=' "$recipe" 2>/dev/null; then
  ok "research_dir set: $(grep -E '^\s*research_dir\s*=' "$recipe" | head -1 | sed -E 's/^[^=]*=\s*//')"
else
  warn "no research_dir in $recipe: research goes to $data/projects/$project_id/research/ (--force installs the repo's: docs/research)"
fi

echo "Claude profiles"
# A profile is a Claude account per project (ADR 0009). Bootstrap never creates one: that needs a
# login, which `dovetail login <name>` runs.
if grep -qE '^\s*\[profiles\]' "$config/config.toml" 2>/dev/null; then
  ok "profiles listed in $config/config.toml (dovetail login --list shows them)"
else
  ok "none: every project uses ~/.claude. For another account: dovetail login <name>, then profile = \"<name>\" in the project's recipe"
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
# `dvtl` is the short name for `dovetail` (ADR 0030 §8); scripts/install.sh makes the symlink.
command -v dvtl  >/dev/null && ok "dvtl (short name for dovetail)" || warn "dvtl not found on PATH (scripts/install.sh installs it next to dovetail)"
command -v nvim  >/dev/null && ok "nvim (notes and worktree hand-off)" || warn "nvim not found (needed later for notes hand-off)"
if [[ "${TERM:-}" == *kitty* || -n "${KITTY_WINDOW_ID:-}" ]]; then
  ok "kitty detected (map image canvas available)"
else
  warn "not running in kitty: maps fall back to the outline view"
fi

echo "Memory caps (systemd-run --user)"
if command -v systemd-run >/dev/null; then
  # The same properties a session gets (dovetail-core `memory`); Dovetail probes again at start.
  if systemd-run --user --scope --quiet --collect -p MemoryMax=64M -p MemorySwapMax=0 \
      -p OOMPolicy=continue true 2>/dev/null; then
    ok "per-session cgroup caps work without root"
  else
    warn "systemd-run --user --scope -p MemoryMax=… failed: sessions will run without memory caps"
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
