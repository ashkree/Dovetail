`dovetail`: the binary crate that wires the workspace crates into one executable.

It opens the terminal UI on `dovetail-core` (default) or `dovetail-mock`, hosts the per-session stdio MCP bridge (`dovetail mcp`), answers Claude Code worktree hooks (`dovetail hook`), and exposes shell utilities (`add`, `login`, `upgrade`).

# Subcommands

| Invocation | Purpose |
| --- | --- |
| `dovetail [--project <id>]` | Default. Opens the TUI on the real `dovetail-core` for the current directory or the named registered project. |
| `dovetail --mock [--seed N] [--speed X]` | Opens the TUI on `dovetail-mock` with virtual time. |
| `dovetail --mock --dump [--seed N] [--count N]` | Prints the mock core's initial snapshot and `N` events as JSON lines to stdout. |
| `dovetail mcp` | Per-session synchronous stdio MCP bridge forwarding tool calls to the running app's unix socket. Dispatched **before** `clap` parses (D42) so startup stays fast and `clap` never writes help or errors to the MCP stdout channel. |
| `dovetail hook <worktree-create\|worktree-remove>` | Claude Code worktree hooks (hidden from top-level `--help`). `worktree-create` creates or reuses the slice/subagent worktree, runs the recipe's setup, records `HookOutcome`, and prints the path; `worktree-remove` is a no-op (Dovetail removes worktrees itself). |
| `dovetail add [--project <id>] <TEXT>...` | Quick-captures a task using `dovetail-tui`'s capture parser; sends it over the running app's socket when live, or writes directly to SQLite. |
| `dovetail login <profile> \| --list` | Creates and signs in a Claude profile (`CLAUDE_CONFIG_DIR`), or lists configured profiles and their login state. |
| `dovetail upgrade [--local]` | Builds `origin/main` (or the local checkout) into the staging directory and either hands it to a running app over its socket or swaps the installed binary in place. |

# Composition

`dovetail` is the only crate that depends on every layer of the workspace, keeping the library boundaries strict:

- **`dovetail-core` + `dovetail-driver`**: `tui::run_real` registers `ClaudeDriver` (and `AgyDriver` when installed) into `Drivers`, spawns the core via `dovetail_core::engine::spawn`, and hands the resulting `CoreLink` to `dovetail-tui`.
- **`dovetail-mock`**: `tui::run_mock` and `main::dump` instantiate `MockCore` behind the same `CoreLink` / `ServerMsg` wire types.
- **`dovetail-tui` + `dovetail-maps`**: `paint.rs` implements `dovetail_tui::canvas::MapPainter` on top of `dovetail_maps::MapRenderer` so `dovetail-tui` stays free of `merman` and `resvg`.
- **`dovetail-tools` + `dovetail-api`**: `mcp/` validates tool calls locally against `dovetail-tools` schemas and forwards `SocketRequest`s over the unix socket; `add`, `hook`, `login`, and `upgrade` share paths, config, storage, and socket helpers from `dovetail-core`.

# Logging and panics

While the TUI owns the terminal, nothing may write to stdout or stderr. `logging::init` (called only for TUI runs, never for `mcp`, `hook`, or `--dump`) sets up daily-rolled file logging to `dovetail.log` under `Paths::state` via `tracing-appender` + `tracing-subscriber` (`RUST_LOG`, default `info`), and installs a panic hook that logs panics to the file before delegating to the previous hook.

# Layout

```text
src/
  main.rs       CLI definition (`clap`), early `dovetail mcp` dispatch, `--dump` runner
  tui.rs        wires `dovetail-core` / `dovetail-mock`, drivers, and `dovetail-tui`; handles project switch and staged upgrade exec on exit
  paint.rs      `MapPainter` adapter over `dovetail_maps::MapRenderer`
  logging.rs    daily file logger (`dovetail.log`) and panic hook under `Paths::state`
  mcp/
    mod.rs      synchronous stdio JSON-RPC 2.0 loop, socket client, per-call worker threads
    schema.rs   `tools/list` definitions, JSON input schemas, and deprecated alias entries
  hook.rs       `dovetail hook worktree-create` and `worktree-remove`
  add.rs        `dovetail add`: quick capture over socket or SQLite
  login.rs      `dovetail login`: Claude profile setup and listing
  upgrade.rs    `dovetail upgrade`: staged release build and binary swap
```
