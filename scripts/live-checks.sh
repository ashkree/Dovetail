#!/usr/bin/env bash
# Live checks: real Claude Code sessions against scratch repos in /tmp. YOU run this; it
# costs a little money (tiny prompts, Haiku where the check allows).
#
#   DOVETAIL_LIVE_TESTS=1 scripts/live-checks.sh            # all checks
#   DOVETAIL_LIVE_TESTS=1 scripts/live-checks.sh driver mcp # some
#
# Checks: driver, permissions, hook, resume, mcp, app (slices, messaging and archive run in app).
# Not covered yet (suggested, by hand in `app`): research rounds and findings, tangents
# (side chats), reading mode, maps, take-over, profiles.
# Everything uses separate XDG dirs under the scratch root, never your real Dovetail state, and
# streams are recorded (DOVETAIL_RECORD_STREAMS=1) so new fixtures can be cut from them.
# Each check prints PASS, FAIL or LOOK (needs your eyes). Logs stay in $ROOT for review.
set -uo pipefail

[[ "${DOVETAIL_LIVE_TESTS:-}" == "1" ]] || { echo "set DOVETAIL_LIVE_TESTS=1 to run live checks"; exit 2; }

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(mktemp -d /tmp/dvt-live.XXXX)"
MODEL="${DOVETAIL_LIVE_MODEL:-haiku}"
export XDG_DATA_HOME="$ROOT/xdg/data" XDG_CONFIG_HOME="$ROOT/xdg/config" XDG_STATE_HOME="$ROOT/xdg/state"
# Short runtime dir: unix socket paths are limited to ~108 bytes.
export XDG_RUNTIME_DIR="$ROOT/r"
export DOVETAIL_RECORD_STREAMS=1
mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME/dovetail/roles" "$XDG_CONFIG_HOME/dovetail/projects" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cp "$repo_root"/config/roles/*.md "$XDG_CONFIG_HOME/dovetail/roles/"

echo "scratch root: $ROOT (model $MODEL)"
(cd "$repo_root" && cargo build -q -p dovetail) || { echo "build failed"; exit 1; }
DT="$repo_root/target/debug/dovetail"

pass() { printf '\033[32mPASS\033[0m %s\n' "$*"; }
fail() { printf '\033[31mFAIL\033[0m %s\n' "$*"; }
look() { printf '\033[33mLOOK\033[0m %s\n' "$*"; }

# A scratch git repo with one commit.
scratch_repo() {
  local dir="$ROOT/$1"
  mkdir -p "$dir" && git -C "$dir" init -q -b main && echo "hello" > "$dir/README" &&
    git -C "$dir" add . && git -C "$dir" -c user.email=t@t -c user.name=t commit -qm init
  echo "$dir"
}

# One headless turn with the flags the core uses; extra flags after the prompt. Stdin stays open for
# HOLD seconds (default 0): closing it right after the prompt aborts permission requests.
turn() {
  local cwd="$1" prompt="$2"; shift 2
  (cd "$cwd" && { printf '{"type":"user","message":{"role":"user","content":%s},"parent_tool_use_id":null}\n' \
    "$(printf '%s' "$prompt" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')"; sleep "${HOLD:-0}"; } |
    timeout 180 claude -p --input-format stream-json --output-format stream-json --verbose \
      --include-partial-messages --permission-prompt-tool stdio --replay-user-messages \
      --model "$MODEL" "$@")
}

check_driver() {
  local repo; repo="$(scratch_repo driver)"
  local id; id="$(uuidgen)"
  turn "$repo" "Reply with exactly the word: alpha" --permission-mode auto --session-id "$id" --name live-driver \
    > "$ROOT/driver.ndjson" 2> "$ROOT/driver.err"
  grep -q '"type":"result"' "$ROOT/driver.ndjson" && grep -q '"permissionMode":"auto"' "$ROOT/driver.ndjson" \
    && pass "driver: streamed turn in auto mode" || fail "driver: see $ROOT/driver.ndjson"
  grep -q '"isReplay":true' "$ROOT/driver.ndjson" && pass "driver: own prompt replayed (isReplay)" \
    || look "driver: no isReplay line; the core skips non-peer replays either way"
}

check_permissions() {
  # Manual mode: the request must reach stdin as can_use_tool. Answered by the in-app check (app).
  local repo; repo="$(scratch_repo perms)"
  HOLD=30 turn "$repo" "Use the Write tool to create x.txt containing hi." --permission-mode default \
    > "$ROOT/perms.ndjson" 2> "$ROOT/perms.err" &
  local pid=$!; sleep 25; kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
  grep -q '"can_use_tool"' "$ROOT/perms.ndjson" && pass "permissions: manual-mode request reaches the host" \
    || fail "permissions: no can_use_tool in $ROOT/perms.ndjson"
}

check_hook() {
  # The WorktreeCreate hook from launch settings, with a setup longer than the default hook timeout (B5).
  local repo; repo="$(scratch_repo hook)"
  printf 'name = "hook"\n[worktree]\nsetup = ["sleep 75"]\n' > "$XDG_CONFIG_HOME/dovetail/projects/hook.toml"
  cat > "$ROOT/hook-settings.json" <<JSON
{"hooks":{"WorktreeCreate":[{"hooks":[{"type":"command","command":"$DT hook worktree-create","timeout":900}]}],
 "WorktreeRemove":[{"hooks":[{"type":"command","command":"$DT hook worktree-remove","timeout":30}]}]}}
JSON
  local id; id="$(uuidgen)"; echo "$id" > "$ROOT/hook.session"
  DOVETAIL_PROJECT=hook turn "$repo" "Remember the codeword BANANA. Reply ok." --permission-mode auto \
    --session-id "$id" --settings "$ROOT/hook-settings.json" --worktree live-slice \
    > "$ROOT/hook.ndjson" 2> "$ROOT/hook.err"
  local wt="$XDG_DATA_HOME/dovetail/projects/hook/worktrees/live-slice"
  [[ -e "$wt/.git" ]] && grep -q "\"cwd\":\"$wt\"" "$ROOT/hook.ndjson" \
    && pass "hook: worktree outside the repo, session runs in it, 75 s setup finished" \
    || fail "hook: see $ROOT/hook.ndjson, $ROOT/hook.err"
  [[ -f "$XDG_STATE_HOME/dovetail/setup/hook/live-slice/hook.json" ]] && pass "hook: outcome recorded" \
    || fail "hook: no hook.json outcome"
  [[ -z "$(git -C "$repo" status --porcelain)" ]] && pass "hook: nothing written into the repo" \
    || fail "hook: repo is dirty: $(git -C "$repo" status --porcelain)"
}

check_resume() {
  # The core resumes a slice with cwd = its worktree and no --worktree (unverified guess until now).
  local id; id="$(cat "$ROOT/hook.session" 2>/dev/null)" || { look "resume: run the hook check first"; return; }
  local wt="$XDG_DATA_HOME/dovetail/projects/hook/worktrees/live-slice"
  turn "$wt" "What was the codeword? Reply with it only." --permission-mode auto --resume "$id" \
    > "$ROOT/resume.ndjson" 2> "$ROOT/resume.err"
  grep -q "BANANA" "$ROOT/resume.ndjson" && pass "resume: worktree session resumed from its worktree" \
    || fail "resume: see $ROOT/resume.ndjson"
  local transcript; transcript="$(find "$HOME/.claude/projects" -name "$id.jsonl" 2>/dev/null | head -1)"
  [[ "$transcript" == *"$(echo "$wt" | sed 's/[^A-Za-z0-9]/-/g')"* ]] \
    && pass "resume: transcript filed under the worktree path, as the core records it" \
    || look "resume: transcript at '$transcript'; compare with session::launch::transcript_path"
}

check_mcp() {
  # The bridge's tools reach a fake-free app: needs the app (see check_app). Here: tools/list per role.
  local out
  out="$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | DOVETAIL_ROLE=slice timeout 5 "$DT" mcp)"
  [[ "$out" == *core_ask_user* && "$out" != *project_start_slice* && "$out" != *project_add* \
    && "$out" != *'"start_slice"'* && "$out" != *tasks_* ]] \
    && pass "mcp: a slice sees no coordinator tools (Core, Coordination, Maps)" || fail "mcp: $out"
  # ADR 0016: for one release the old names are listed too, pointing at the new ones.
  [[ "$out" == *'"name":"ask_user"'* && "$out" == *'Call `core_ask_user` instead'* ]] \
    && pass "mcp: old names still listed as deprecated aliases" || fail "mcp: $out"
  out="$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | DOVETAIL_ROLE=side-chat DOVETAIL_SUITES=core timeout 5 "$DT" mcp)"
  [[ "$out" == *core_log_decision* && "$out" != *maps_draw* && "$out" != *research_* ]] \
    && pass "mcp: a side chat sees Core only" || fail "mcp: $out"
  # Stdin stays open a moment: on end of input the bridge exits without waiting for its answer.
  out="$({ printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"core_report_status","arguments":{"status":"x"}}}'; sleep 1; } |
    DOVETAIL_ROLE=coordinator DOVETAIL_SOCKET="$ROOT/none.sock" timeout 5 "$DT" mcp)"
  [[ "$out" == *"not running"* ]] && pass "mcp: app absent gives a clear error, no hang" || fail "mcp: $out"
}

check_app() {
  # The full app, by hand: these need the TUI and your judgement.
  local repo; repo="$(scratch_repo app)"
  printf 'name = "app"\n[worktree]\nsetup = []\n' > "$XDG_CONFIG_HOME/dovetail/projects/app.toml"
  printf 'default_model = "%s"\nmax_slices = 2\n' "$MODEL" > "$XDG_CONFIG_HOME/dovetail/config.toml"
  look "app: open the app with:  (cd $repo && XDG_DATA_HOME=$XDG_DATA_HOME XDG_CONFIG_HOME=$XDG_CONFIG_HOME XDG_STATE_HOME=$XDG_STATE_HOME XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR $DT)"
  cat <<STEPS
  Then, and note each result:
   1. Status bar says "not a Dovetail project yet". Run :project add; the warning goes.   (ADR 0005)
   2. i, "Reply with the word pong", Enter: the reply streams in.                          (M6)
   3. i, "Run sleep 30 with Bash", then <leader>sx (interrupt): "Interrupted" appears.     (M6)
   4. <leader>sm to manual. i, "Create a.txt with the Write tool": a Permission item
      appears; answer y. Ask for two writes in one message; answer the second first.      (M7)
   5. "Call core_ask_user: which colour?": in the chat and Needs you; reply r.            (ADR 0007)
   6. "Call core_report_status with 'testing'": the line shows in the tree.               (M9)
   7. "Create a task 'demo' with tasks_create, then project_start_slice on it named
      demo-1 with brief 'reply ok'": Permission item (manual mode); y; slice sets up; the coordinator
      gets "slice demo-1 ready".                                                          (M8, ADR 0004)
   8. Coordinator: "SendMessage to demo-1: say hi back". Message timeline (<leader>m) shows
      both directions.                                                                    (M8)
   9. In the slice: "make a commit on your branch". Then dd on the slice row: a
      "Remove worktree" item asks because of the unpushed commit; y removes it all.
      git -C $repo branch --list 'dovetail/*' is empty; git status is clean.               (M8)
  10. "Run: gh pr merge 1" in the coordinator in auto mode: a Permission item should
      appear (the ask rule, B7); deny it. If it runs without asking, record the B7 fallback.
  11. :q. ps shows no claude processes left.                                              (M6)
  Recorded streams: $XDG_STATE_HOME/dovetail/run/*/*/stream-*.ndjson
STEPS
}

checks=("$@"); [[ ${#checks[@]} -eq 0 ]] && checks=(driver permissions hook resume mcp app)
for c in "${checks[@]}"; do
  case "$c" in
    driver) check_driver ;; permissions) check_permissions ;; hook) check_hook ;;
    resume) check_resume ;; mcp) check_mcp ;; app) check_app ;;
    *) echo "unknown check $c" ;;
  esac
done
echo "logs and recordings: $ROOT"
