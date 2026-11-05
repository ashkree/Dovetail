# Wave 1 review and retro

Review of `pre-wave-1...wave-1` (the contracts commit, then tracks A storage, B driver, C worktrees and D MCP bridge, about 5,300 lines) with `/code-review`. Both axes were run separately by Opus reviewers. Date: 2026-11-05.

**Result:** no hard standards violations. Under the wave-1 rule, only `cargo build --workspace` was run (it passes); no tests were written or run, and clippy was not run. Smoke checks:
- **Real core, in a scratch repo with separate XDG directories:**
  - a task added with `dovetail add` while the app was closed shows on open;
  - a task added while the app is running appears live through the socket;
  - `:q` exits cleanly and removes the socket.
- **MCP bridge, against a fake app socket:**
  - a call is forwarded with the agent id and `toolUseId`;
  - bad arguments are rejected locally;
  - with no app running, the agent gets a clear error.
- **Driver and worktree hook:** not run; they wait for the live checks.

## Standards

| Finding | Kind | Outcome |
| --- | --- | --- |
| No `unwrap`/`expect`/`unsafe`; `anyhow` only in the binary; children reaped; SQLite only on the DB thread; the bridge is std-only and writes only JSON-RPC to stdout; tokio only in core and driver; nothing written into repos but branches | Documented standards | Clean |
| `socket::TaskSink::add_task` returns `Result<_, String>` | "typed errors in libraries", soft | **Wave 2** |
| `service.rs` holds an unread `config` with `#[allow(dead_code)]` | Speculative Generality | **Wave 2** reads it (`max_slices`) |
| `projects::identify` and `worktree::main_checkout` both find the main checkout, by different rules | Duplicated Code, could disagree | **Wave 2**: one function |
| The NDJSON socket client is written twice (`socket::try_add_task`, `mcp::roundtrip`) | Duplicated Code | Kept: the bridge must stay std-only and small, and the two are about 30 lines each |
| Tool names are matched as strings in three places in `mcp.rs` | Repeated Switches | **Fixed** in the retro: `dovetail_api::tools::Tool` |
| The `DOVETAIL_*` env var names are literals in `hook.rs` and `mcp.rs` | Shotgun Surgery risk | **Fixed** in the retro: `dovetail_api::tools::env` |
| `(paths, project, name)` travel together through `worktree.rs`; the project id is a bare string | Data Clumps | Open |

## Spec

| Finding | Kind | Outcome |
| --- | --- | --- |
| The bridge docs told the core to pass `--strict-mcp-config`; B28 says not to | Wrong | **Fixed** |
| The `create_task` schema offered bucket `now`; ADR 0002 says "never straight into Now" | Wrong | **Fixed**: `next` and `later` only |
| `NewTask.notes` was required in serde but optional in the tool schema | Mismatch | **Fixed**: `#[serde(default)]` |
| The coordinator prompt hard-coded "at most 3", but `max_slices` is configurable | Mismatch | **Fixed** |
| `resolve` registers an unknown repo silently; B25 says "offers to register" | Deviation | **Decided**: [ADR 0005](../adr/0005-explicit-projects-any-folder.md), built in wave 2 |
| `map_user` drops `origin`, `isReplay` and the raw JSON | Blocks the M8 message timeline (S2) | **Wave 2** |
| With no upstream and no remote, every removal counts the whole history as unpushed and asks | Safe but noisy | **Wave 2**: count only commits after the branch point |
| `ToolHandler` gets no access to core state; `create_task` hard-codes `coordinator_created = false` | Makes wave 2 harder | **Wave 2** |
| `start_slice` `name` is not checked against `worktree::validate_name`; an `agent-` prefix would skip setup | Bug in waiting | **Wave 2** |
| After an app restart, a bridge's first tool call fails before it reconnects | Matters for M10 | **Wave 2**: reconnect once before failing |
| Role is enforced only in the bridge | ADR 0004: "Slices cannot call it" | **Wave 2**: the core refuses too |
| M5 boxes ticked in a build-only wave | Unverified | Restart survival and live `add` are confirmed by the smoke check above; the recipe and bootstrap were read, not run |
| `SystemdScope`, the fixture recorder and `Session::kill` | Scope creep, minor | Kept: small, and M13 needs the scope |
| `--resume` with `--worktree` is a guess | Unverified | Live checks |

## Retro

Run 2026-11-05. Ordered by severity; the outcome is the user's call.

| Finding | Category | Outcome |
| --- | --- | --- |
| The worst bugs were mismatches between tracks (string vs integer ids, `p0` vs `P0`, env var literals, `--strict-mcp-config`): the contracts commit covered the socket messages but not tool inputs or env names | Coding standards / process | **Done:** tool inputs (`Tool`, `AskUser`, ..., `StartSlice`), `Role` and the `env` names live in `dovetail_api::tools`; the bridge parses and validates into them and the socket carries the typed `Tool`. The plan's Waves section now says every cross-track value goes in the contracts commit |
| No guardrail: no pre-commit hook, no CI | Automated checks | **Later:** the project is in its dev phase |
| Track agents ticked boxes they never verified | Process | **Done:** a Waves rule; only the integrator ticks, after checking |
| Track D could not drive its own bridge against a socket; the scratchpad path is too long for a unix socket | Information access | **Asked:** explained to the user |
| The M5–M9 "Tests:" lists contradict "tests when something breaks" | Relevance | **Done:** marked deferred, kept for when tests return |
| Build output filtered through `grep` hid whether compilation ran | Tool economy | Agent habit: `cargo build 2>&1 \| tail -3` |
