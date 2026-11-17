#!/usr/bin/env bash
# Build the GUI with the perf harness and run its scenarios once, in the real window (WebKitGTK).
# The harness (gui/src/perf/) swaps in a synthetic core with a lot of data, presses keys, streams
# deltas and zooms the canvas, and the window reports one JSON line per scenario through the
# shell (`PERF {...}` on stderr). Method and numbers: docs/plan/gui-perf.md.
#
#   scripts/perf-gui.sh            # build (no LTO: it keeps the build under a 3 GB cap), run, print
#   scripts/perf-gui.sh --no-build # run the last perf build again
#
# Uses a throwaway state dir under /tmp; never touches ~/.local/share/dovetail. Needs a display.
set -euo pipefail
cd "$(dirname "$0")/.."
out="${PERF_OUT:-/tmp/dovetail-perf.txt}"

if [[ "${1:-}" != "--no-build" ]]; then
    cap='{"app":{"security":{"capabilities":[{"identifier":"perf","windows":["main"],"permissions":["core:default","allow-core-send","allow-core-subscribe","allow-perf-log"]}]}}}'
    (cd gui && VITE_PERF=1 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
        CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
        npx tauri build --no-bundle --features perf --config "$cap")
fi

state="$(mktemp -d /tmp/dovetail-perf-state.XXXXXX)"
export DOVETAIL_DATA_DIR="$state/data" DOVETAIL_CONFIG_DIR="$state/config" \
    DOVETAIL_STATE_DIR="$state/state" DOVETAIL_RUNTIME_DIR="$state/run"
mkdir -p "$DOVETAIL_DATA_DIR" "$DOVETAIL_CONFIG_DIR" "$DOVETAIL_STATE_DIR" "$DOVETAIL_RUNTIME_DIR"
target/release/dovetail-gui >"$out" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true; rm -rf "$state"' EXIT
waitfor() { for _ in $(seq 1 180); do grep -q "$1" "$out" && return; sleep 1; done; }
procs() { echo "$pid"; pgrep -P "$pid"; }
ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
# CPU over 5 s and memory (proportional set size, so shared libraries count once) of the app and
# its WebKit child processes; the window's pages run in WebKitWebProcess.
sample() {
    declare -A before
    for p in $(procs); do before[$p]=$(ticks "$p"); done
    sleep 5
    for p in $(procs); do
        cpu=$(( ($(ticks "$p") - before[$p]) * 100 / 5 / $(getconf CLK_TCK) ))
        echo "PROC $1 $(cat /proc/$p/comm) pss_mb=$(awk '/^Pss:/{print int($2/1024)}' /proc/$p/smaps_rollup) rss_mb=$(awk '/VmRSS/{print int($2/1024)}' /proc/$p/status) cpu_pct=$cpu" >>"$out"
    done
}
for n in 1 2 3 4 5 6 7; do
    for _ in $(seq 1 120); do [[ $(grep -c '"phase"' "$out") -ge $n ]] && break; grep -q '"done"' "$out" && break; sleep 1; done
    name=$(grep '"phase"' "$out" | sed -n "${n}p" | sed 's/.*"phase":"\([^"]*\)".*/\1/')
    [[ -n "$name" ]] || break
    sleep 1; sample "$name"
done
waitfor '"done"'
grep -E '^(PERF|PROC)' "$out"
