#!/usr/bin/env bash
# Probe agy's stream-json print mode. Usage: stream.sh <name> <stdin-file> [agy flags...]
# Writes out/<name>.ndjson (stdout) and out/<name>.err (stderr). Runs in /tmp/agy-spike/repo.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
name="$1"; input="$(realpath "$2")"; shift 2
mkdir -p "$here/out"
cd /tmp/agy-spike/repo
timeout "${PROBE_TIMEOUT:-120}" agy --input-format stream-json --output-format stream-json "$@" -p="" \
  < "$input" > "$here/out/$name.ndjson" 2> "$here/out/$name.err"
echo "exit=$?"
