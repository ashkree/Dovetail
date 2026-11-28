`dovetail-tools`: the agent-to-core MCP tool catalogue, argument validation, and unix-socket RPC messages.

Everything an agent session or CLI helper sends to the Dovetail core over the per-project unix socket — the [`TOOLS`] catalogue, [`Role`] and [`Suite`] gating, [`Tool`] argument parsing and validation, launch environment variable names ([`env`](mod@env)), and the [`SocketRequest`] / [`SocketResponse`] wire types — lives here as plain Rust data. Like `dovetail-api`, this crate has no I/O, no threads, and no async runtime: it depends only on `dovetail-api`, `serde`, and `serde_json`.

# Dependency boundary

`dovetail-tools` sits between `dovetail-api` and the runtime side of Dovetail:

- **Depends on `dovetail-tools`**: `dovetail-core` (tool handlers, session launch config, socket server) and the `dovetail` binary (`dovetail mcp`, `dovetail hook`, `dovetail add`, `dovetail upgrade`).
- **Must NOT depend on `dovetail-tools`**: `dovetail-tui` and `dovetail-mock`. Clients only speak the UI-to-core protocol in `dovetail-api` (`Snapshot`, `Event`, `Command`), never agent tools or socket RPC. Cargo enforces this boundary (`cargo tree -p dovetail-tui | grep dovetail-tools` must stay empty).

# The domain map

Tools are grouped by **domain** (a folder) and then by **feature** (a file, one per suite). The domains are the same as in `dovetail-api`.

| Domain | Features (suite) | What it is |
|---|---|---|
| `tracking/` | `task` (tasks), `project` (project), `coordination` (coordination) | The user's work: tasks, slices and projects, effort reports |
| `agents/` | `report` (core) | What an agent says about itself: questions, status, decisions |
| `visual/` | `canvas`, `map` (maps), `design` | Things drawn |
| `research.rs` | `research` | Research rounds and reports (one feature, so a plain file) |

`catalog.rs`, `error.rs`, `env.rs`, `socket.rs` and `lib.rs` stay at the top. The `report` feature is the `core` suite; it is called `report` because a module named `core` clashes with Rust's built-in `core`.

## Paths and re-exports

The full path is `domain::feature::Type`, for example `dovetail_tools::tracking::task::ListTasks`. `lib.rs` also re-exports every feature module at the crate root, so `dovetail_tools::task::ListTasks` is the same type. Use the short path in callers.

# The feature pattern

Each feature is one file, `<domain>/<feature>.rs`, holding:

- its `<Feature>Tool` enum (`#[serde(rename_all = "snake_case")]`), one variant per tool;
- the input structs for those tools;
- `pub(crate) fn name(&self)` and `pub(crate) fn validate(&self, role: Role)` for the checks serde cannot express.

```text
src/
  lib.rs        module list, this document, root re-exports, and the top-level Tool enum
                (Tool::parse, Tool::allowed, Tool::validate, Tool::name)
  catalog.rs    Role, Suite, ToolSpec, the TOOLS table, tool_spec(), deprecated_alias(),
                deprecation_note(), tools_in()
  error.rs      ToolError and the shared field checks
  env.rs        names of the DOVETAIL_* environment variables passed to bridges and hooks
  socket.rs     SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody
                (NDJSON over the project unix socket)
  tracking/     task.rs, project.rs, coordination.rs
  agents/       report.rs
  visual/       canvas.rs, map.rs, design.rs
  research.rs
tests/
  tools.rs      parsing, deprecated alias, validation, and SocketRequest wire tests
```

# Parsing, validation, gating, and deprecated aliases

- **Wire shape ([`SocketRequestBody::ToolCall`]).** [`Tool`] is a nested enum with `#[serde(rename_all = "snake_case")]`, wrapping each feature's own `snake_case` `<Suite>Tool` enum. On the unix socket, [`SocketRequestBody::ToolCall`] serialises `call` as a nested externally-tagged object such as `{"task":{"list":{..}}}`.
- **Catalogue and deprecated aliases ([`catalog`]).** [`TOOLS`] is the single source of truth for tool names, their owning [`Suite`], and optional `old_name` aliases from before ADR 0016. [`tool_spec`] matches either the current name (`<suite>_<verb>`) or `old_name`. [`deprecated_alias`] identifies old names so `dovetail mcp` can forward `called_as` and the core can append [`deprecation_note`] to the tool result.
- **[`Tool::parse`] and [`Tool::validate`].** `dovetail mcp` calls [`Tool::parse`] with the tool name and MCP JSON arguments, looking up [`tool_spec`] and deserialising into the matching suite variant (`ToolError::Unknown` or `ToolError::BadArguments` on failure). It then calls [`Tool::validate`] with the caller's [`Role`] to enforce checks serde cannot express (non-empty strings, positive ids, `tasks_create` rejecting `now`, `core_log_decision` requiring a task for non-slice roles, slice-only `coordination_report` and `design_declare_variants`). `dovetail-core` checks role and suite rules again on receipt so a hand-written socket client cannot bypass the bridge.
- **Role and suite gating.** [`Role::default_suites`] defines the default suites per role (for example, `Coordinator` gets `Suite::ALL`; `Slice` gets `Core`, `Coordination`, `Maps`, `Design`). At launch, `dovetail-core` writes `DOVETAIL_ROLE` ([`env::ROLE`]) and `DOVETAIL_SUITES` ([`env::SUITES`], formatted via [`Suite::join`]). The bridge parses them with [`Role::from_env_value`] and [`Suite::parse_list`], filters `tools/list` with [`tools_in`], and enforces `tools/call` with [`Tool::allowed`].

# How to add a tool

1. Add the input struct and a new variant to `<Feature>Tool` in `src/<domain>/<feature>.rs`. Update its `name()` and `validate()` arms.
2. Add a [`ToolSpec`] row to [`TOOLS`] in `src/catalog.rs` and a match arm in [`Tool::parse`] in `src/lib.rs`.
3. Handle the new variant in `dovetail-core` (`crates/dovetail-core/src/mcp/tools.rs`) and add its MCP description and JSON `inputSchema` in `crates/dovetail/src/mcp/schema.rs` (`tool_defs()`, which a test asserts stays in sync with [`TOOLS`]).
4. Add parsing, round-trip and validation tests in `tests/tools.rs`.

# How to add a feature (suite)

1. Add the variant to [`Suite`] in `src/catalog.rs` (including `Suite::ALL`, `Suite::as_str`, and [`Role::default_suites`] if a role gets it by default).
2. Create `src/<domain>/<feature>.rs` as described above, add `pub mod <feature>;` to the domain's `mod.rs` and a `pub use <domain>::<feature>;` to `src/lib.rs`. A new domain is a folder with a `mod.rs`; a domain with one feature can be a plain file, as `research.rs` is.
3. Add a `<Feature>(<Feature>Tool)` variant to [`Tool`] in `src/lib.rs`, wiring its `name()`, `parse()` and `validate()` arms.
4. Register the tools in [`TOOLS`], `dovetail-core`, `crates/dovetail/src/mcp/schema.rs` and `tests/tools.rs` as above.
