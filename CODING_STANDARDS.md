# Coding standards — Dovetail

The rules a review checks code against. The hard constraints in `AGENTS.md` apply too; this file holds the detail behind them. `scripts/check.sh` enforces the mechanical ones.

## Architecture

- **Crates mirror the architecture.** A plain-data **API/protocol** crate (`dovetail-api`: snapshot, events, commands, `apply()` reducer); the **tools** crate (`dovetail-tools`: MCP tool catalogue and unix-socket RPC between agents and core); the **core** (sessions, storage, worktrees, server side of the MCP bridge); a **mock core** implementing the same API; the **TUI**; the `dovetail` binary (subcommands: TUI default, `mcp`, `add`). The exact split is in `docs/implementation-plan.md` §1.
- **Core / UI boundary is message-passing.** The core sends one `Snapshot`, then deltas; the UI owns its copy and applies them with the shared reducer; the UI sends `Command`s. Types are plain data with `serde` derives, so the core can later become a daemon over a unix socket without changing the TUI. The UI reaches the core only through this boundary.
- **`SessionDriver` trait** hides the Claude Code crate entirely: start/resume, send, interrupt, set permission mode, answer control (permission) requests, event stream. A **fake driver** replays recorded NDJSON fixtures for tests.
- **Protocol JSON is parsed in `dovetail-driver`.** The core receives typed driver events and never parses stream or control JSON itself.
- **Stream rules from the spikes are law** (digest §3–§6): route subagent events by `parent_tool_use_id`; treat a session as done only after its final `result`, since the first one may not be; answer permission requests concurrently; record session ids and transcript paths yourself; attribute to-do items from the stream; clean up worktrees yourself.
- **One action registry.** Every key and `:` command resolves to one `Action` enum through one data-like keymap table (mode + buffer kind + key sequence → action). Movement is global, actions are buffer-local.
- **Buffers in regions.** Any buffer can sit in any region. One reusable picker component. One theme struct holds every colour.
- **Storage** goes through one DB thread with typed requests; schema changes go through migrations; only the core talks to SQLite.
- **MCP bridge:** `dovetail mcp` is a stdio MCP server that connects back to the running app over a unix socket and forwards tool calls; tool handlers live in the core.

## Code

- Cargo workspace under `crates/`; shared lints in the root `Cargo.toml` (`unsafe_code = "forbid"`; any exception needs the user's OK).
- Errors: typed (`thiserror`) in libraries, `anyhow` at the binary edge.
- **Destructive paths fail loudly.** A parse or IO failure on a path that deletes or overwrites returns a typed error, never a default value.
- `unwrap()`/`expect()` only in tests and genuinely impossible states, with a comment saying why. One deliberate exception: `Action::DebugPanic` (`:panic`, debug builds only) panics on purpose to exercise the panic hook.
- Child processes are always reaped; the terminal is restored on exit and on panic (panic hook).
- While the TUI runs, all output goes through `tracing` to `~/.local/state/dovetail/` (or the project's `logs/`); stdout and stderr stay silent.
- Doc comments on every public type in the API crate.
- **One source for each limit or constant.** User-facing text that names a limit (a cap, a timeout, a count) reads it from the same constant the code uses.

## Maintainability

The project is maintained for the long term, so code is judged on how easy it is to change later, not only on whether it works.
- **Reuse before writing.** Search for an existing function, type, component or helper that does the job or nearly does. Extend it rather than adding a parallel one.
- **Consolidate what you touch.** Two pieces of code doing the same thing in the area a change touches become one. Consolidation outside that area is proposed to the coordinator as its own task, never slipped into another change.
- **Extension points only where the roadmap names the next step.** For example, ADR 0030's order of work says the page and the daemon come next, so clients stay behind the `Snapshot`/`Command` boundary. No generic layers, plugin hooks or configuration for needs nobody has named.
- **Say what was reused.** A slice's `done` report names what it reused or extended, or why the code had to be new. The review checks that claim.
- Spikes and other throwaway code are exempt.

## Readability (the user has poor eyesight: a requirement, not polish)

120-column reading column centred in wide panes; blank rows between paragraphs, list items, code blocks and headings; markdown keeps bold/italic/code styling and each also gets its own colour; off-white body text, never pure white; tool activity folded into one summary line per turn; thinking hidden; long output truncated with "show more"; no solid backgrounds on areas (panes, columns, views): the selection highlight and badges are the only exceptions (ADR 0018 amendment, 2026-11-11). Details in `docs/ui-session-brief.md`.

## Testing

When tests are due is in `AGENTS.md` ("Which skill when"). What they cover:

- Unit tests: mode state machine, keymap resolution (`gg`, `dd`, `]]`, `<leader>ss`, timeouts, ambiguity), reducer, quick-capture parser, `maps_draw` validation, task-list JSON parsing (including mid-write parse errors).
- Snapshot tests: ratatui `TestBackend` + `insta`, from deterministic mock data (seeded, time-free).
- Driver/core tests: replay recorded NDJSON fixtures from `tests/fixtures/` through the fake driver (subagent routing, double `result`, concurrent permission requests).
- Storage tests: temp-dir databases, migrations up from empty.
- Live tests only with `DOVETAIL_LIVE_TESTS=1`.

## Pull requests

- **Choices beyond the brief are questions.** Behaviour a slice chose that its brief did not ask for is listed under "Questions" in its PR, so the user decides.
