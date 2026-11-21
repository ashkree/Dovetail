#!/usr/bin/env bash
# The user's run of the tui-readability spike (ADR 0030 §5, kitty-text-sizing folded in).
# Run it from a kitty window, wide enough to be your normal working size:
#     spikes/tui-readability/run.sh            # about 20 minutes
# Detection runs first. Then each step says what to look at, waits for Enter, opens the spike;
# `q` ends the step. Take screenshots where it says so, with your usual tool.
# Never commit the screenshots or anything in fixtures/ and out/: they hold real transcript text.
set -euo pipefail
cd "$(dirname "$0")"
bin=./target/release/tui-readability-spike
mkdir -p out

[[ -x $bin ]] || cargo build --release
[[ -d fixtures && -n $(ls fixtures 2>/dev/null) ]] || ./fetch-fixtures.sh
if [[ $TERM != xterm-kitty ]]; then
    echo "Run this from kitty (TERM is '$TERM')." >&2
    exit 1
fi
fixtures=(v2-rework tui-return skills-grill)
variants=(A B C)
# Part 1: two heading steps. Part 2: every variant on every fixture.
total=$((2 + ${#fixtures[@]} * ${#variants[@]}))
n=0

block() { # block "label" "item1|item2"; prints nothing when there are no items
    local label=$1 items=$2
    local -a arr=()
    [[ -n $items ]] || return 0
    IFS='|' read -ra arr <<<"$items"
    printf '%s\n' "$label"
    local item
    for item in "${arr[@]}"; do printf '  - %s\n' "$item"; done
}

step() { # step "title" "what" "checks" "tries" "shots" command...
    local title=$1 what=$2 checks=$3 tries=$4 shots=$5
    shift 5
    n=$((n + 1))
    printf '\n\033[1m== Step %d of %d: %s\033[0m\nWhat this is: %s\n' "$n" "$total" "$title" "$what"
    block "Check:" "$checks"
    block "Try:" "$tries"
    block "Screenshot:" "$shots"
    read -r -p "Enter to start (s to skip) " ans
    [[ $ans == s ]] && return 0
    "$@" || true
}

part_banner() { # part_banner "Part name" "what the part decides"
    printf '\n\033[1m### %s\033[0m\n%s\n' "$1" "$2"
}

keys() {
    cat <<'EOF'
Keys:
  scroll         j/k lines, d/u half pages, g/G top/bottom
  focus wide     Tab/Shift-Tab focus a table or code block
  pan            h/l 8 cols, 0/$ start/end
  variant        1/2/3 variant A/B/C
  toggles        t next transcript, S solid bg, H highlighting, O toggle OSC 66
  quit           q
EOF
}

# Detection: no prompt. The result is saved and summarised in one line.
printf '\n\033[1mDetection (saved to out/probe-kitty.txt)\033[0m\n'
"$bin" --probe 2>&1 | tee out/probe-kitty.txt || true
if grep -q 'osc66 detect: ON' out/probe-kitty.txt && grep -q '0 mismatches' out/probe-kitty.txt; then
    echo "Detection OK"
else
    echo "Detection problem: see above; scaled headings may not work"
fi

part_banner "Part 1: scaled headings" "Decides whether scaled headings (OSC 66) ship on kitty, and what the fallback looks like."
keys
step "Scaled headings (variant B)" \
    "transcript v2-rework, H1 drawn at 2x size, H2 at 1.5x." \
    "H1 is double size with a ━ rule under it; H2 is 1.5x|Scrolling (j, d, u): headings move cleanly, no leftover cells, no stripes, no flicker|A heading cut off at the top or bottom edge is drawn at normal size|Resizing the window redraws the headings correctly" \
    "O  turn scaled headings off, then O again to turn them back on" \
    "one H1, one H2, one cut-off heading" \
    "$bin" --variant b --fixture fixtures/v2-rework.jsonl
step "Fallback headings (variant B, no scaling)" \
    "v2-rework, OSC 66 off: bold headings with rules; nothing else changes." \
    "Headings are bold with rules, at normal size|Prose, code tint and tables look as they did in step 1" \
    "" "" \
    "$bin" --variant b --no-osc66 --fixture fixtures/v2-rework.jsonl

part_banner "Part 2: compare variants" "Decides which variant (A, B or C) and which mix reads best on real transcripts."
echo "Shared checklist, every step below:"
block "Check:" "Prose width, margins and spacing look right|Headings look right|Tables fit; a table wider than the pane is clipped with ▸|Code has its tint and highlighting|Box-drawn diagrams never wrap|A wide table pans with Tab, then l/h|S switches solid and transparent; both look right"
block "Try:" "Scroll to a spot with a wide table and a code block in view, at your usual window size"
block "Screenshot:" "one per variant per transcript, at that spot"
for f in "${fixtures[@]}"; do
    for v in "${variants[@]}"; do
        step "Variant $v on $f" \
            "The checklist above, at the spot you chose; screenshot it." \
            "" "" "" \
            "$bin" --variant "${v,,}" --fixture "fixtures/$f.jsonl"
    done
done

cat <<'EOF'

Done. Please report back with:
  - the screenshots (kept private), and out/probe-*.txt;
  - for each of:
      - variant (variant A, B or C, or a mix)
      - OSC 66 headings in or out
      - sideways-scroll keys (Tab, h/l ok?)
  - anything that flickered, ghosted or mis-aligned.
EOF
