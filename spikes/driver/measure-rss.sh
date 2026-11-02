#!/usr/bin/env bash
# Peak RSS (VmHWM) of a spike binary run against the fake claude.
# Usage: measure-rss.sh <binary> [args...]
here=$(cd "$(dirname "$0")" && pwd)
export PATH="$here/fake-claude:$PATH"
export FAKE_FIXTURE=${FAKE_FIXTURE:-$here/../../crates/dovetail-driver/tests/fixtures/plain_turn.ndjson}
export SPIKE_OUT=${SPIKE_OUT:-/tmp/dovetail-spikes/driver/out-fake}
"$@" > /dev/null 2>&1 &
pid=$!; peak=0
while kill -0 $pid 2>/dev/null; do
  v=$(awk '/VmHWM/ {print $2}' /proc/$pid/status 2>/dev/null); [ -n "$v" ] && peak=$v
  sleep 0.05
done
echo "$(basename "$1") peak RSS: ${peak} kB"
