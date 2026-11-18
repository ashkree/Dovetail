#!/usr/bin/env bash
# The user's run of the tui-readability spike (ADR 0030 §5, kitty-text-sizing folded in).
# Run it from a kitty window, wide enough to be your normal working size:
#     spikes/tui-readability/run.sh            # everything, about 20 minutes
#     spikes/tui-readability/run.sh kitty      # phase 1 and 2 only, no tmux
#     spikes/tui-readability/run.sh tmux       # phase 3 only
# Each step says what to look at, waits for Enter, opens the spike; `q` ends the step. Take
# screenshots where it says so, with your usual tool.
# Never commit the screenshots or anything in fixtures/ and out/: they hold real transcript text.
set -euo pipefail
cd "$(dirname "$0")"
phase=${1:-all}
bin=./target/release/tui-readability-spike
mkdir -p out

[[ -x $bin ]] || cargo build --release
[[ -d fixtures && -n $(ls fixtures 2>/dev/null) ]] || ./fetch-fixtures.sh
if [[ $TERM != xterm-kitty && $phase != tmux ]]; then
    echo "Run this from kitty (TERM is '$TERM'). In tmux, run: $0 tmux" >&2
    exit 1
fi
fixtures=(v2-rework tui-return skills-grill)

step() { # step "title" "what to look at" command...
    local title=$1 look=$2
    shift 2
    printf '\n\033[1m== %s\033[0m\n%s\n' "$title" "$look"
    read -r -p "Enter to start (s to skip) " ans
    [[ $ans == s ]] && return 0
    "$@" || true
}

keys() {
    cat <<'EOF'
Keys: j/k d/u g/G scroll, Tab/Shift-Tab focus a table/code block, h/l pan 8 cols, 0/$ start/end,
      1 2 3 variant A/B/C, t next transcript, S solid bg, H highlighting, O toggle OSC 66, q quit.
EOF
}

if [[ $phase == all || $phase == kitty ]]; then
    echo "Phase 1: kitty-text-sizing (variant B, headings at scale)"
    keys
    step "1a. Detection" \
        "Look at: osc66 detect: ON, and both raw-probe answers true. Width check: any glyph mismatches are listed." \
        bash -c "$bin --probe 2>&1 | tee out/probe-kitty.txt"
    step "1b. Scaled headings, kitty" \
        "Look at: H1 at 2x with a ━ rule, H2 at 1.5x. Scroll (j, d, u): do headings scroll in and out cleanly, with no ghost cells, no stripe left behind, no flicker? Stop with a heading half cut at the top or bottom edge: is it normal size there? Resize the window while it runs: do headings redraw? Press O (OSC off) and O again. SCREENSHOT: one H1, one H2, one cut heading." \
        "$bin" --variant b --fixture fixtures/v2-rework.jsonl
    step "1c. The fallback" \
        "Look at: --no-osc66 on B: bold headings with rules, nothing else changes." \
        "$bin" --variant b --no-osc66 --fixture fixtures/v2-rework.jsonl

    echo; echo "Phase 2: variants x fixtures (kitty)"
    for f in "${fixtures[@]}"; do
        for v in a b c; do
            step "2. variant ${v^^} on $f" \
                "Scroll to a spot with a wide table and a code block in view, at your usual window size. Judge: prose width, margins, spacing, headings, table fit, code tint and highlighting, diagrams (box-drawn) never wrapped, a table wider than the pane clipped with ▸ and panning with Tab, l/h. Press S to compare solid and transparent. SCREENSHOT at that spot (one per variant per transcript)." \
                "$bin" --variant "$v" --fixture "fixtures/$f.jsonl"
        done
    done
fi

if [[ $phase == all || $phase == tmux ]]; then
    echo; echo "Phase 3: kitty under tmux (a private tmux server; your tmux is untouched)"
    command -v tmux >/dev/null || { echo "tmux is not installed" >&2; exit 1; }
    in_tmux() { # in_tmux passthrough cmd...
        local pt=$1
        shift
        tmux -L dvtl-spike -f /dev/null kill-server 2>/dev/null || true
        tmux -L dvtl-spike -f /dev/null new-session -d -s spike "$*; sleep 1"
        tmux -L dvtl-spike set -g allow-passthrough "$pt"
        tmux -L dvtl-spike attach -t spike
    }
    for pt in off on; do
        step "3a. Probe under tmux, allow-passthrough $pt" \
            "Look at: detect OFF (tmux). The forced raw probe shows what tmux does with OSC 66: record the two answers." \
            in_tmux "$pt" "$bin --probe --force-osc66 2>&1 | tee out/probe-tmux-$pt.txt; sleep 8"
    done
    step "3b. Default under tmux (OSC 66 off)" \
        "Look at: variant B must look like its fallback: bold headings, tinted highlighted code. Check tables and diagrams: do box-drawing glyphs and ━ line up? Press 1 2 3 to compare variants. SCREENSHOT variant B at a table + code spot." \
        in_tmux off "$bin --variant b --fixture fixtures/v2-rework.jsonl"
    for pt in on off; do
        step "3c. OSC 66 forced through tmux, allow-passthrough $pt" \
            "Look at: do scaled headings appear at all? Scroll with j/d/u: any garbage, stale or doubled headings, stripes, a corrupted layout that does not recover on redraw (try resizing)? That is the desync the proposal predicts. SCREENSHOT whatever happens." \
            in_tmux "$pt" "$bin --variant b --force-osc66 --fixture fixtures/v2-rework.jsonl"
    done
fi

cat <<'EOF'

Done. Please report back with:
  - the screenshots (kept private), and out/probe-*.txt;
  - for each of: variant (A/B/C or a mix), OSC 66 headings in or out, sideways-scroll keys (Tab, h/l ok?);
  - anything that flickered, ghosted or mis-aligned.
EOF
