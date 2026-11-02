#!/usr/bin/env bash
# Spike S4: does `systemd-run --user --scope` actually enforce memory caps?
# Throwaway. Needs: systemd --user, cgroup v2 with memory delegated, python3 (optional; falls back to `tail /dev/zero`).
# Safe to run any time: every cap is 200M and every scope is a short-lived one named dovetail-s4-*.
# Prints one PASS/FAIL/INFO line per check. Exit status is 1 if any check FAILed.
set -u

TAG="dovetail-s4-$$"
T="$(mktemp -d "${TMPDIR:-/tmp}/dovetail-s4.XXXXXX")"
UID_N="$(id -u)"
FAILS=0
N=0

scrub() { sed -e "s#${HOME}#~#g" -e "s#user-${UID_N}#user-UID#g" -e "s#user@${UID_N}#user@UID#g"; }
pass() { echo "PASS  $1: $2" | scrub; }
fail() { echo "FAIL  $1: $2" | scrub; FAILS=$((FAILS + 1)); }
info() { echo "INFO  $1: $2" | scrub; }
# File-backed counter: $(unit) runs in a subshell, so a plain variable would never advance and every scope got the same name.
unit() { local n; n=$(( $(cat "$T/n" 2>/dev/null || echo 0) + 1 )); echo "$n" >"$T/n"; echo "${TAG}-$n"; }
now_ms() { echo $(($(date +%s%N) / 1000000)); }

cleanup() {
  systemctl --user stop "${TAG}"-*.scope >/dev/null 2>&1
  systemctl --user reset-failed "${TAG}"-*.scope >/dev/null 2>&1
  rm -rf "$T"
}
trap cleanup EXIT

# ---- prerequisites -------------------------------------------------------
for c in systemd-run systemctl cmp; do
  command -v "$c" >/dev/null || { echo "FAIL  prerequisites: $c not found"; exit 2; }
done
if ! systemd-run --user --scope --quiet --collect -p MemoryMax=64M true 2>"$T/pre.err"; then
  echo "FAIL  prerequisites: scope did not start: $(cat "$T/pre.err")"; exit 2
fi
info "env" "$(systemd-run --version | head -1); python3=$(command -v python3 >/dev/null && echo yes || echo no)"

# Eater: grows 10 MiB every 20 ms, touching the pages. Slow enough to poll memory.events.
if command -v python3 >/dev/null; then
  cat >"$T/eater.py" <<'PY'
import time
b = []
while True:
    b.append(b"x" * (10 * 1024 * 1024))
    time.sleep(0.02)
PY
  EATER=(python3 "$T/eater.py")
else
  EATER=(tail /dev/zero)
fi

# watch_events UNIT PID OUTFILE: poll the scope's memory.events from outside until PID exits.
# The scope's cgroup disappears on exit, so the last successful read is kept in OUTFILE.
watch_events() {
  local u="$1" pid="$2" out="$3" cg="" f
  : >"$out"
  while kill -0 "$pid" 2>/dev/null; do
    if [ -z "$cg" ]; then
      cg="$(systemctl --user show -p ControlGroup --value "$u.scope" 2>/dev/null)"
      [ -n "$cg" ] && echo "$cg" >"$out.cg"
    fi
    f="/sys/fs/cgroup${cg}/memory.events"
    if [ -n "$cg" ] && [ -r "$f" ]; then
      cat "$f" >"$out.tmp" 2>/dev/null && [ -s "$out.tmp" ] && mv "$out.tmp" "$out"
    fi
    sleep 0.01
  done
}
oom_kills() { awk '$1=="oom_kill"{print $2}' "$1" 2>/dev/null | head -1; }

# ---- 1. memory eater under the cap ---------------------------------------
U="$(unit)"
# systemd 262 defaults to OOMPolicy=stop: after a kernel OOM kill it SIGTERMs the whole scope at once and removes the cgroup,
# so the counters vanish. OOMPolicy=continue lets us read the kernel's own oom_kill count (default-policy behaviour is tested in check 2).
# The wrapper shell outlives the eater by 1 s, so the cgroup (and its final memory.events) stays readable after the kill.
systemd-run --user --scope --quiet --collect --unit="$U" -p MemoryMax=200M -p MemorySwapMax=0 -p OOMPolicy=continue \
  --setenv=OUT="$T/c1.status" -- sh -c '"$@"; echo $? >"$OUT"; sleep 1' sh "${EATER[@]}" 2>/dev/null &
P=$!
watch_events "$U" "$P" "$T/ev1"
wait "$P"
ST="$(cat "$T/c1.status" 2>/dev/null || echo 0)"
OOM="$(oom_kills "$T/ev1")"
if [ ! -s "$T/ev1" ]; then
  fail "1 memory-cap-kills-eater" "exit=$ST but memory.events was never readable (memory controller not delegated to the scope?). cg=$(cat "$T/ev1.cg" 2>/dev/null)"
elif [ "$ST" -gt 128 ] && [ "${OOM:-0}" -ge 1 ]; then
  pass "1 memory-cap-kills-eater" "exit=$ST (signal $((ST - 128))), oom_kill=$OOM"
else
  fail "1 memory-cap-kills-eater" "exit=$ST oom_kill=${OOM:-none}; last memory.events: $(tr '\n' ' ' <"$T/ev1")"
fi

# ---- 2. forked child over the cap ----------------------------------------
U="$(unit)"
systemd-run --user --scope --quiet --collect --unit="$U" -p MemoryMax=200M -p MemorySwapMax=0 -p OOMPolicy=continue \
  --setenv=OUT="$T/fork.out" -- \
  sh -c '
    "$@" & c=$!
    cat /proc/$$$$/cgroup >"$OUT.p"
    cat /proc/$c/cgroup >"$OUT.c" 2>/dev/null
    wait $c; st=$?
    echo "child_status=$st" >"$OUT"
    echo "parent_survived=yes" >>"$OUT"
    sleep 1
  ' sh "${EATER[@]}" 2>/dev/null &
P=$!
watch_events "$U" "$P" "$T/ev2"
wait "$P"; ST=$?
OOM="$(oom_kills "$T/ev2")"
CST="$(sed -n 's/^child_status=//p' "$T/fork.out" 2>/dev/null)"
PC="$(cat "$T/fork.out.p" 2>/dev/null)"
CC="$(cat "$T/fork.out.c" 2>/dev/null)"
if grep -q '^parent_survived=yes' "$T/fork.out" 2>/dev/null; then SURV="parent SURVIVED (child_status=${CST:-?})"; else SURV="parent DIED too (scope exit=$ST)"; fi
if [ -n "$PC" ] && [ "$PC" = "$CC" ] && [ "${OOM:-0}" -ge 1 ]; then
  pass "2 forked-child-counts-against-cap" "child shares parent's cgroup, oom_kill=$OOM; $SURV"
else
  fail "2 forked-child-counts-against-cap" "parent_cg='${PC:-?}' child_cg='${CC:-?}' oom_kill=${OOM:-none}; $SURV"
fi
info "2 parent-fate-continue" "OOMPolicy=continue: $SURV"
# Same fork test under systemd's DEFAULT policy: does the parent survive the child being OOM-killed?
systemd-run --user --scope --quiet --collect --unit="$(unit)" -p MemoryMax=200M -p MemorySwapMax=0 \
  --setenv=OUT="$T/fork2.out" -- \
  sh -c '"$@" & c=$!; wait $c; echo "child_status=$?" >"$OUT"; echo parent_survived=yes >>"$OUT"; sleep 1' sh "${EATER[@]}" 2>/dev/null
ST2=$?
if grep -q '^parent_survived=yes' "$T/fork2.out" 2>/dev/null; then SURV2="parent SURVIVED"; else SURV2="parent DIED too (scope exit=$ST2; 143 = systemd SIGTERMed the scope)"; fi
info "2 parent-fate-default" "default OOMPolicy: $SURV2 (decides whether M13 sees a dead session or a live one with a dead tool; pass -p OOMPolicy=continue to keep the parent)"

# ---- 3. NDJSON through stdin/stdout --------------------------------------
{
  for i in 1 2 3; do printf '{"type":"user","n":%d,"text":"h\\u00e9llo \xe2\x9c\x93 \\"quoted\\" tab\\t"}\n' "$i"; done
  printf '{"type":"big","data":"%s"}\n' "$(head -c 200000 /dev/zero | tr '\0' 'a')"
  printf '{"type":"last"}\n'
} >"$T/in.ndjson"
systemd-run --user --scope --quiet --collect --unit="$(unit)" -p MemoryMax=200M -- cat <"$T/in.ndjson" >"$T/out.ndjson" 2>"$T/out.err"
RC=$?
if [ "$RC" -eq 0 ] && cmp -s "$T/in.ndjson" "$T/out.ndjson"; then
  R1="bulk bytes identical ($(wc -c <"$T/in.ndjson") B, 5 lines)"
else
  R1=""
  fail "3 ndjson-passthrough" "bulk copy differs or failed (rc=$RC) stderr=$(head -c 200 "$T/out.err")"
fi
# Interactive round trip: a line written must come back without closing stdin (no buffering).
if [ -n "$R1" ]; then
  coproc ECHO { systemd-run --user --scope --quiet --collect --unit="$(unit)" -p MemoryMax=200M -- \
    sh -c 'while IFS= read -r l; do printf "%s\n" "$l"; done' 2>"$T/echo.err"; }
  printf '{"type":"ping","n":1}\n' >&"${ECHO[1]:-2}" 2>/dev/null
  if IFS= read -r -t 5 -u "${ECHO[0]}" LINE && [ "$LINE" = '{"type":"ping","n":1}' ]; then
    pass "3 ndjson-passthrough" "$R1; live line echoed back unchanged before stdin closed"
  else
    fail "3 ndjson-passthrough" "$R1; but no live echo within 5s (got '${LINE:-nothing}'; stderr: $(head -c 200 "$T/echo.err" 2>/dev/null))"
  fi
  [ -n "${ECHO[1]:-}" ] && eval "exec ${ECHO[1]}>&-" 2>/dev/null
  wait "${ECHO_PID:-}" 2>/dev/null
fi

# ---- 4. PID and exit status ----------------------------------------------
systemd-run --user --scope --quiet --collect --unit="$(unit)" -- sh -c 'echo $$$$ >"$1"; exit 7' sh "$T/pid" 2>/dev/null &
P=$!
wait "$P"; ST=$?
CPID="$(cat "$T/pid" 2>/dev/null)"
if [ "$ST" -eq 7 ]; then
  if [ "$CPID" = "$P" ]; then
    pass "4 pid-and-exit-status" "exit status 7 visible; command PID == caller's \$! ($P), so kill/wait on the handle hits the session"
  else
    pass "4 pid-and-exit-status" "exit status 7 visible; NOTE command PID $CPID != caller's \$! $P (driver must use the real PID)"
  fi
else
  fail "4 pid-and-exit-status" "expected exit 7, got $ST (command pid=${CPID:-?}, handle pid=$P)"
fi
# A signal death should surface as 128+n too.
systemd-run --user --scope --quiet --collect --unit="$(unit)" -- sh -c 'kill -TERM $$$$' 2>/dev/null; SST=$?
info "4 signal-status" "child killed by SIGTERM seen by caller as exit=$SST (143 expected)"

# ---- 5. launch overhead --------------------------------------------------
declare -a WITH WITHOUT
for i in 1 2 3 4 5 6 7; do
  s=$(now_ms); true; e=$(now_ms); WITHOUT+=($((e - s)))
  s=$(now_ms); systemd-run --user --scope --quiet --collect --unit="$(unit)" -p MemoryMax=200M -- true 2>/dev/null; e=$(now_ms); WITH+=($((e - s)))
done
MED_W="$(printf '%s\n' "${WITH[@]}" | sort -n | sed -n 4p)"
MED_B="$(printf '%s\n' "${WITHOUT[@]}" | sort -n | sed -n 4p)"
if [ "$MED_W" -lt 200 ]; then
  pass "5 launch-overhead" "median ${MED_W} ms over 7 launches (baseline plain launch ${MED_B} ms; samples: ${WITH[*]})"
else
  fail "5 launch-overhead" "median ${MED_W} ms >= 200 ms (baseline ${MED_B} ms; samples: ${WITH[*]})"
fi

# ---- 6. scope cleanup ----------------------------------------------------
# Also check a scope that was OOM-killed WITHOUT --collect, to learn whether failed scopes linger.
U="$(unit)"
systemd-run --user --scope --quiet --unit="$U" -p MemoryMax=100M -p MemorySwapMax=0 -- "${EATER[@]}" 2>/dev/null; NC=$?
sleep 0.5
if systemctl --user list-units --type=scope --all --no-legend "$U.scope" 2>/dev/null | grep -q "$U"; then
  info "6 no-collect" "OOM-killed scope (exit=$NC) LINGERS without --collect: M13 must pass --collect or reset-failed"
  systemctl --user reset-failed "$U.scope" >/dev/null 2>&1
else
  info "6 no-collect" "OOM-killed scope (exit=$NC) is gone even without --collect"
fi
LEFT=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
  LEFT="$(systemctl --user list-units --type=scope --all --no-legend "${TAG}-*.scope" 2>/dev/null | awk '{print $1}' | tr '\n' ' ')"
  [ -z "$LEFT" ] && break
  sleep 0.2
done
STRAY="$(pgrep -f "dovetail-s4.*eater" 2>/dev/null | tr '\n' ' ')"
if [ -z "$LEFT" ] && [ -z "$STRAY" ]; then
  pass "6 scope-cleanup" "no ${TAG}-* scope units and no stray eater processes after exit (all runs used --collect)"
else
  fail "6 scope-cleanup" "left over scopes: ${LEFT:-none}; stray pids: ${STRAY:-none}"
fi

echo
if [ "$FAILS" -eq 0 ]; then echo "S4 summary: all checks passed"; else echo "S4 summary: $FAILS check(s) FAILED"; fi
[ "$FAILS" -eq 0 ]
