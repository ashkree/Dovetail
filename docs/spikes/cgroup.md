# S4: cgroup enforcement

**Status:** run 2026-11-02, all six checks pass (see Result). Owner: you (needs your machine and user systemd).

`spikes/cgroup/run.sh` checks that `systemd-run --user --scope` enforces a memory cap, not only that a scope starts. It is throwaway. Every cap is 200M and every scope is named `dovetail-s4-*` with `--collect`, so it is safe to run any time.

Run it and paste the output into this file under "Result":

```
bash spikes/cgroup/run.sh
```

It prints one `PASS`/`FAIL` line per check plus `INFO` lines, and exits 1 if any check fails. A check never passes without evidence (for example `oom_kill` is read from the scope's `memory.events` by polling from outside while the scope lives, since the cgroup disappears on exit).

| # | Check | Pass |
| --- | --- | --- |
| 1 | Eater under `MemoryMax=200M`, `MemorySwapMax=0` | Exit above 128 and `oom_kill` >= 1 |
| 2 | Forked child over the cap | Parent and child share a cgroup and `oom_kill` >= 1; whether the parent survives is recorded as INFO |
| 3 | NDJSON through stdin/stdout | Bulk file (including a 200 kB line) is byte-identical, and a live line echoes back before stdin closes |
| 4 | PID and exit status | Caller sees exit 7; INFO on whether the command PID equals the caller's `$!` and on signal status |
| 5 | Launch overhead | Median of 7 launches under 200 ms (baseline printed) |
| 6 | Scope cleanup | No `dovetail-s4-*` scope or stray eater left; INFO on whether an OOM-killed scope lingers without `--collect` |

Uses `python3` for the eater if present, otherwise `tail /dev/zero`.

**Fallback if 1 or 2 fail:** `MemoryHigh` throttling or `prlimit`; failing that, ship memory display only and warn.

## Result

Run by the user on 2026-11-02, systemd 262, cgroup v2, after two script fixes (see below). All six checks PASS; no fallback needed.

```
PASS  1 memory-cap-kills-eater: exit=137 (signal 9), oom_kill=1
PASS  2 forked-child-counts-against-cap: child shares parent's cgroup, oom_kill=1; parent SURVIVED (child_status=137)
INFO  2 parent-fate-continue: OOMPolicy=continue: parent SURVIVED (child_status=137)
INFO  2 parent-fate-default: default OOMPolicy: parent DIED too (scope exit=143; systemd SIGTERMed the scope)
PASS  3 ndjson-passthrough: bulk bytes identical (200230 B, 5 lines); live line echoed back unchanged before stdin closed
PASS  4 pid-and-exit-status: exit status 7 visible; command PID == caller's $!, so kill/wait on the handle hits the session
INFO  4 signal-status: child killed by SIGTERM seen by caller as exit=143
PASS  5 launch-overhead: median 6 ms over 7 launches (baseline plain launch 1 ms)
INFO  6 no-collect: OOM-killed scope (exit=137) LINGERS without --collect
PASS  6 scope-cleanup: no dovetail-s4-* scope units and no stray eater processes after exit
S4 summary: all checks passed
```

Findings for later milestones (M13 memory caps):
- The kernel kills only the biggest process in the scope. systemd's default `OOMPolicy=stop` then SIGTERMs the whole scope (exit 143), so a Claude session dies with its tool. Pass `-p OOMPolicy=continue` if Dovetail should see a live session with a dead tool.
- Pass `--collect` (or `reset-failed`): an OOM-killed scope otherwise lingers.
- `systemd-run` treats `$$` in arguments as an escape for a literal `$`. Avoid `$$` in launch arguments, or write `$$$$`.
- The scope's PID equals the caller's `$!`, so signals and `wait` on the handle reach the session.

First two runs failed on script bugs (not real results): scope names did not advance (subshell counter), `$$` was escaped by systemd, and the cgroup vanished before `oom_kill` could be read. The kernel log showed the cap enforced throughout.

## Open

None.
