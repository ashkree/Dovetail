#!/usr/bin/env bash
# WorktreeCreate hook for the S1 spike: idempotent, prints the worktree path.
# Also records the env and hook input so the spike can see what reached it.
set -euo pipefail
input=$(cat)
base=/tmp/dovetail-spikes/driver
name=$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')
cwd=$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin)["cwd"])')
printf '%s\n' "$input" >> "$base/hook-input.log"
printf 'DOVETAIL_SPIKE_MARK=%s\n' "${DOVETAIL_SPIKE_MARK:-unset}" >> "$base/hook-env.log"
wt="$base/worktrees/$name"
if [ ! -d "$wt" ]; then
  mkdir -p "$base/worktrees"
  git -C "$cwd" worktree add -q -b "dovetail/$name" "$wt" >&2
fi
printf '%s\n' "$wt"
