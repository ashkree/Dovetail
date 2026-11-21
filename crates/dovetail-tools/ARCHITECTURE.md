`dovetail-tools`: the agent-to-core MCP tool catalogue, argument validation, and unix-socket RPC messages.

Everything an agent session or CLI helper sends to the Dovetail core over the per-project unix socket — the [`TOOLS`] catalogue, [`Role`] and [`Suite`] gating, [`Tool`] argument parsing and validation, launch environment variable names ([`env`](mod@env)), and the [`SocketRequest`] / [`SocketResponse`] wire types — lives here as plain Rust data. Like `dovetail-api`, this crate has no I/O, no threads, and no async runtime: it depends only on `dovetail-api`, `serde`, and `serde_json`.

# Dependency boundary

`dovetail-tools` sits between `dovetail-api` and the runtime side of Dovetail:

- **Depends on `dovetail-tools`**: `dovetail-core` (tool handlers, session launch config, socket server) and the `dovetail` binary (`dovetail mcp`, `dovetail hook`, `dovetail add`, `dovetail upgrade`).
- **Must NOT depend on `dovetail-tools`**: `dovetail-tui` and `dovetail-mock`. Clients only speak the UI-to-core protocol in `dovetail-api` (`Snapshot`, `Event`, `Command`), never agent tools or socket RPC. Cargo enforces this boundary (`cargo tree -p dovetail-tui | grep dovetail-tools` must stay empty).

# Layout

```text
src/
  lib.rs          module list, #![doc = include_str!("../ARCHITECTURE.md")], top-level Tool
                  enum (Tool::parse, Tool::allowed, Tool::validate, Tool::name), and root
                  re-exports
  catalog.rs      Role, Suite, ToolSpec, TOOLS table, tool_spec(), deprecated_alias(),
                  deprecation_note(), and tools_in()
  error.rs        ToolError (typed error implementing Display and std::error::Error) and
                  shared field checks
  env.rs          names of DOVETAIL_* environment variables passed to bridges and hooks
  socket.rs       SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody
                  (NDJSON framing over the project unix socket)
  <suite>/        8 suite feature folders, each holding its <Suite>Tool enum, input structs,
                  and per-suite validation:
    core/mod.rs           CoreTool (core_ask_user, core_report_status, core_log_decision)
    task/mod.rs           TaskTool, BucketFilter (tasks_create, tasks_list, tasks_get,
                          tasks_update, tasks_archive)
    project/mod.rs        ProjectTool, archive_result (project_start_slice,
                          project_archive_slice, project_add)
    coordination/mod.rs   CoordinationTool, ReportState (coordination_report,
                          coordination_claim_criterion)
    research/mod.rs       ResearchTool (research_start_round, research_write_report,
                          research_propose, research_get)
    map/mod.rs            MapTool (maps_draw; raw JSON checked by dovetail-maps in core)
    canvas/mod.rs         CanvasTool (canvas_add_block, canvas_add_card, canvas_link_cards)
    design/mod.rs         DesignTool (design_declare_variants)
tests/
  tools.rs        tool parsing, deprecated alias, validation, and SocketRequest wire tests
```

# Parsing, validation, gating, and deprecated aliases

- **Wire shape ([`SocketRequestBody::ToolCall`]).** [`Tool`] is a nested enum with `#[serde(rename_all = "snake_case")]`, wrapping each suite's own `snake_case` `<Suite>Tool` enum. On the unix socket, [`SocketRequestBody::ToolCall`] serialises `call` as a nested externally-tagged object such as `{"task":{"list":{..}}}`.
- **Catalogue and deprecated aliases ([`catalog`]).** [`TOOLS`] is the single source of truth for tool names, their owning [`Suite`], and optional `old_name` aliases from before ADR 0016. [`tool_spec`] matches either the current name (`<suite>_<verb>`) or `old_name`. [`deprecated_alias`] identifies old names so `dovetail mcp` can forward `called_as` and the core can append [`deprecation_note`] to the tool result.
- **[`Tool::parse`] and [`Tool::validate`].** `dovetail mcp` calls [`Tool::parse`] with the tool name and MCP JSON arguments, looking up [`tool_spec`] and deserialising into the matching suite variant (`ToolError::Unknown` or `ToolError::BadArguments` on failure). It then calls [`Tool::validate`] with the caller's [`Role`] to enforce checks serde cannot express (non-empty strings, positive ids, `tasks_create` rejecting `now`, `core_log_decision` requiring a task for non-slice roles, slice-only `coordination_report` and `design_declare_variants`). `dovetail-core` checks role and suite rules again on receipt so a hand-written socket client cannot bypass the bridge.
- **Role and suite gating.** [`Role::default_suites`] defines the default suites per role (for example, `Coordinator` gets `Suite::ALL`; `Slice` gets `Core`, `Coordination`, `Maps`, `Design`). At launch, `dovetail-core` writes `DOVETAIL_ROLE` ([`env::ROLE`]) and `DOVETAIL_SUITES` ([`env::SUITES`], formatted via [`Suite::join`]). The bridge parses them with [`Role::from_env_value`] and [`Suite::parse_list`], filters `tools/list` with [`tools_in`], and enforces `tools/call` with [`Tool::allowed`].

# Adding a tool or suite

## Adding a tool to an existing suite

1. Add the input struct (and any helper types) and a new variant to `<Suite>Tool` in `src/<suite>/mod.rs`. Update its `name()` and `validate()` arms.
2. Add a [`ToolSpec`] row to [`TOOLS`] in `src/catalog.rs` and a match arm in [`Tool::parse`] in `src/lib.rs`.
3. Handle the new variant in `dovetail-core` (`crates/dovetail-core/src/tools.rs`) and add its MCP description and JSON `inputSchema` in `crates/dovetail/src/mcp.rs` (`tool_defs()`, which a test asserts stays in sync with [`TOOLS`]).
4. Add parsing, round-trip, and validation tests in `tests/tools.rs`.

## Adding a new suite

1. Add the variant to [`Suite`] in `src/catalog.rs` (including `Suite::ALL`, `Suite::as_str`, and [`Role::default_suites`] if a role gets it by default).
2. Create `src/<suite>/mod.rs` with `<Suite>Tool` (`#[serde(rename_all = "snake_case")]`), `pub(crate) fn name(&self)`, `pub(crate) fn validate(&self, role: Role)`, and the tool input structs.
3. Add `pub mod <suite>;` and a new `<Suite>(<Suite>Tool)` variant to [`Tool`] in `src/lib.rs`, wiring its `name()`, `parse()`, and `validate()` arms.
4. Register the suite's tools in [`TOOLS`], `dovetail-core`, `crates/dovetail/src/mcp.rs`, and `tests/tools.rs` as above.
