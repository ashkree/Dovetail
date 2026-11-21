# Plan: agy driver and driver switching (task #46)

**For:** any agent finishing task #46, with no earlier context. Read this file, then
`docs/spikes/agy-headless.md` (the spike: protocol facts, JSON shapes, the design) and
`crates/dovetail-driver/ARCHITECTURE.md` (the driver crate as built). Both are the reference;
this plan does not repeat them.

## Goal

When Claude Code hits its usage limit, the user keeps working on **agy** (Antigravity CLI,
`~/.local/bin/agy`, v1.5.9), which also offers Claude Opus 5.5 on Google's quota.

1. agy runs as a second headless session driver beside Claude Code, behind the same
   `SessionDriver`/`Session` traits and the same `DriverEvent` vocabulary.
2. The model switcher becomes a telescope picker (the one `Picker`) listing models grouped by
   driver. Picking another driver's model asks for confirmation, then starts a new coordinator
   session on that driver through the `:fresh` flow.
3. A usage menu (`:usage`, and `/usage` typed in the chat bar) shows the limits and current
   usage of both drivers.

Decisions (task #46's log; ask the coordinator for `tasks_get 46` if you need the wording):

- **D33**: drive agy over ACP (`agy acp`, JSON-RPC 2.0 on stdio), one process per session, each
  with its own `--cli_dir_name` settings folder holding `{"toolPermission":"request-review"}`
  so permission requests reach Dovetail.
- **D34**: switching driver is a new session in the same cwd, seeded through `:fresh` with
  Dovetail's facts and the old transcript's path. Never a live switch: neither CLI can load the
  other's history, and a Claude session at its limit cannot write a handoff.
- **D35**: only the **coordinator** switches driver. New **slices** may be started on agy.
  The picker confirms a switch: **handoff** is the default, **clean start** the alternative.
  agy's settings folder lives under Dovetail's run dir; agy's own logs stay in `~/.gemini`.

**Out of scope:** coordinator → agy-slice messages (task #53; agy cannot receive Claude Code's
`SendMessage`). Switching a running slice's driver. Any MCP menu for agy (ACP has none).

## Done so far

| Commit | Where | What |
| --- | --- | --- |
| `f5d2955` | main | Spike: `docs/spikes/agy-headless.md`, probes in `spikes/agy-headless/` |
| `37ae2fb` | main | Step A: driver crate split into `claude/`, `process.rs`; `ClaudeOptions` on `SessionSpec`; `dovetail_api::DriverKind` |
| `9727ac8` | main | Step B: the agy ACP driver (`crates/dovetail-driver/src/agy/`), `AgyOptions`, `McpServerSpec`, `FakeDriver::for_agy`, fixtures in `tests/fixtures/agy/`, `tests/agy_live.rs` |
| C1 commit (see `git log --grep "Usage readers"`) | main | Step C1: the usage readers. The `agy-wip` branch (`bf6bd30`) was its WIP and is superseded |

## Rules

- Small modules, one subject each. Reuse before writing new code; consolidate duplication in
  what you touch. Follow `CODING_STANDARDS.md` and `AGENTS.md`.
- No `ts` attributes on new api types (ts-rs is shelved, task #52).
- Keep `crates/dovetail-driver/ARCHITECTURE.md` current with every driver change.
- Library code returns `thiserror` errors, never `String` (`check.sh` enforces it).
- New dependencies need a row in `docs/implementation-plan.md` §2 (`jiff` already has one; the
  driver was added to it on `agy-wip`).
- Never run real sessions against the user's projects. Live tests sit behind
  `DOVETAIL_LIVE_TESTS=1` and use scratch folders under the target dir. Run the TUI for manual
  checks with `scripts/dev.sh`, which keeps state under `target/dovetail-dev/`.
- **Main must always work**: the user runs Dovetail from it. Each step: rebase on
  `origin/main`, run `CARGO_BUILD_JOBS=2 scripts/check.sh` until it prints
  `check.sh: all checks passed`, then `git push origin HEAD:main`. Unfinished work goes only to
  the `agy-wip` branch (`git push origin HEAD:agy-wip`), never to main.
- Commit messages include `(task #46)`. Tick this checklist in the same push as the step.
- A new migration takes the next number after main's last (now `0022_slice_queue_model.sql` in
  `crates/dovetail-core/src/db/migrations/state/`).

## Steps

### C1. Usage readers in the driver crate

- [x] Finish and land what is on `agy-wip` (`bf6bd30`). It holds:
  - `dovetail_api::{DriverUsage, UsageGroup, LimitWindow}` in `crates/dovetail-api/src/driver.rs`
    (`DriverUsage { driver, plan, groups, note, failed }`, `DriverUsage::failed(driver, why)`;
    `used_fraction` is 0–1).
  - `SessionDriver::usage(&self, UsageQuery) -> UsageFuture` in `src/lib.rs`
    (`UsageFuture = Pin<Box<dyn Future<Output = DriverUsage> + Send>>`; never fails, a failed
    read is `failed: true` with the reason in `note`). `UsageQuery { cwd, env }` in
    `src/query.rs`.
  - `src/query.rs`: `ask(program, args, &query, input lines, keep_stdin, pick)` runs a
    short-lived process, returns the first stdout line `pick` accepts (or the whole stdout as
    one document), kills it, 30 s timeout. `process::spawn_in(program, args, cwd, env)` was split
    out of `process::spawn`.
  - `src/claude/usage.rs`: `claude -p --input-format stream-json --output-format stream-json
    --verbose --no-session-persistence`, sent `initialize` then
    `{"type":"control_request","request_id":"dovetail_usage","request":{"subtype":"get_usage"}}`,
    no prompt, so no model call. The answer's `response.response.rate_limits` has
    `five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet` (each
    `{utilization: percent, resets_at: RFC 3339}` or null) and `extra_usage`; shape in the
    spike's Evidence → Usage, full sample in `spikes/agy-headless/out/c1-usage.ndjson`.
  - `src/agy/usage.rs`: `agy --output-format json -p=/usage` → `command.data.groups[]
    {name, description, buckets[] {window: 5h|weekly, remaining_fraction, reset_time}}`; sample
    in `spikes/agy-headless/out/p-usage.json`.
  - `FakeDriver::set_usage`, `FakeDriver::usage_queries`.
  - Unit tests for both parsers and for `query::ask`; `tests/usage_live.rs` (opt-in; it passed
    against the real CLIs on 2026-11-21).
- [x] Fix the one `check.sh` failure: `query::ask` returns `Result<Value, String>`. Add a
  `QueryError` (`thiserror`: `Spawn(DriverError)`, `Io(std::io::Error)`, `NoAnswer`,
  `Timeout(u64)`) and have both `usage()` functions turn it into `DriverUsage::failed(kind,
  err.to_string())`.
- Verify: `cargo test -p dovetail-driver`; once:
  `DOVETAIL_LIVE_TESTS=1 cargo test -p dovetail-driver --test usage_live -- --nocapture`
  prints a Claude group (5-hour, Weekly) and three agy groups.

### C2. Api additions (`crates/dovetail-api`)

- [x] `Agent.driver: DriverKind` (`#[serde(default)]`, so old data reads as `Claude`) in
  `src/model.rs`.
- [x] `ModelOption.driver: DriverKind` (`#[serde(default)]`), so the picker can group.
  `ModelOption::named` keeps `Claude`; add `ModelOption::named_on(driver, name)` if needed.
  `model_option_index` must match only options of the session's own driver.
- [x] `Command::SwitchDriver { agent: AgentId, driver: DriverKind, model: String, effort:
  Option<ReasoningEffort>, handoff: bool }` in `src/protocol.rs`. Coordinator only; the core
  refuses it for any other agent with `Event::CommandFailed`.
- [x] `Command::LoadUsage` and `Event::UsageLoaded(Vec<DriverUsage>)`. `apply()` leaves the
  snapshot unchanged for `UsageLoaded` (the menu holds the report, like `McpReply`).
- [x] `tools::StartSlice.driver: Option<DriverKind>` (`#[serde(default)]`) so the coordinator can
  start a slice on agy; document that `model` then names an agy family
  (`claude-opus-5-5`, `gemini-3.8-flash`).
- Tests: serde round trips with and without the new fields; `apply` cases.

### C3. Core: a driver per session (`crates/dovetail-core`)

- [ ] **Drivers set.** Replace `CoreOptions.driver: Arc<dyn SessionDriver>`
  (`src/service.rs`) with `drivers: Drivers`, a small type in a new `src/drivers.rs`: a
  `BTreeMap<DriverKind, Arc<dyn SessionDriver>>` with `get(kind) -> Option<&Arc<..>>` and
  `Drivers::single(driver)` for tests. The binary (`crates/dovetail/src/tui.rs`) registers
  `ClaudeDriver::with_program(..)` and `AgyDriver::new()`. Update every test that builds
  `CoreOptions` (grep `driver: Arc::new(`).
- [ ] **Store the driver.** Migration `0023_agent_driver.sql`:
  `ALTER TABLE agents ADD COLUMN driver TEXT NOT NULL DEFAULT 'claude';` plus `AgentRow.driver`
  read and written in `src/db/sessions.rs` (use `DriverKind::as_str`/`parse`). Also add it to
  the slice queue if a queued slice can be on agy (`0022_slice_queue_model.sql` shows the shape).
- [ ] **Launch split.** Move `src/session/launch.rs` to `src/session/launch/mod.rs` (the shared
  `Launch`, the bridge env, `allowed_tools`) with `launch/claude.rs` (today's `spec()` body:
  `settings.json`, `mcp.json`, `ClaudeOptions`) and `launch/agy.rs`:
  - the per-session settings folder `paths.run_dir(project, agent).join("agy")` holding
    `settings.json` = `{"toolPermission":"request-review"}` → `spec.agy.settings_dir`;
  - `spec.agy.mcp_servers = [McpServerSpec { name: "dovetail", command: exe, args: ["mcp"],
    env: <the same bridge env as mcp.json> }]`;
  - `spec.agy.allowed_tools`: Dovetail's tools as agy names them, `mcp_dovetail_<tool>` for
    each name `allowed_tools(&suites)` yields for the `dovetail` server (agy's own allow rules do
    not cover MCP tools; spike a17/a18);
  - side chats: `PermissionMode::DontAsk` (the agy driver then denies anything not allowed);
  - no `CLAUDE_CONFIG_DIR`, no worktree flag.
  `launch::spec(paths, launch, driver)` picks the module.
- [ ] **Starting.** In `Core::launch` (`src/session/mod.rs`, the `self.options.driver.start(spec)`
  call) start on `self.options.drivers.get(row.driver)`. For agy:
  - the Claude-home checks (`claude_home`, the "moved" profile check) do not apply;
  - resume: Claude resumes when `transcript_path` is set. agy has no transcript file Dovetail
    reads, so record `transcript_path = ~/.gemini/antigravity-cli/conversations/<agy id>.db`
    when the first `Init` arrives (the path the switch seed and `:fresh` notice name), and
    resume when it is set;
  - **session id (stream rule 4):** agy picks its own id and reports it in the first `Init`.
    In `on_init` (`src/session/mod.rs`), for an agy session store `init.session_id` as the
    row's `session_id` instead of warning "session id differs";
  - `on_init` sets `turn.running = true` because Claude Code sends `Init` per turn. agy sends
    it once, at open, before any prompt: for agy do not mark a turn running there.
- [ ] **Slices on agy.** `SliceRequest` (`src/slice.rs`) gains `driver`. Claude Code creates a
  slice's worktree through `--worktree` and the `WorktreeCreate` hook; agy has no such flag, so
  for agy call `worktree::create(&paths, &CreateRequest{..})` (`src/worktree.rs`, the function
  the hook calls) on a blocking task first, then launch agy with the worktree as cwd. The
  `project_start_slice` handler passes `StartSlice.driver`.
- [ ] **Models per driver.** `on_models_listed` (`src/session/model.rs`) replaces one
  `model_catalog`; an agy session's `DriverEvent::Models` would overwrite Claude's. Keep
  `model_catalog: BTreeMap<DriverKind, Vec<ModelInfo>>`, filled from the session's driver.
  `model_options()` returns Claude's options (from `config.toml` names, as today) followed by
  agy's families with `driver: Agy`. Before any agy session has run, agy's list is empty:
  optionally fill it at start from `agy models` (check its output format first; skip if it
  needs a model call).
- [ ] **SwitchDriver.** A handler that checks the agent is the coordinator and no turn runs, sets
  the coordinator row's `driver` and `model_choice` to the new driver and model, then runs the
  existing flow: `handoff: true` → `handoff_start()`, `false` → `fresh_session()`
  (`src/journal.rs`). The new session starts on the new driver because launch reads the row.
  With handoff: if the old session cannot answer (a Claude session at its limit), the journal
  block already falls back to the facts and says why (`NoHandoff`); make sure the seed then
  names the old transcript's path (`fresh_notice` does for the notice) so the agy session can
  read it. `SetModel` for a model of the session's own driver is unchanged.
- [ ] **LoadUsage.** For each driver in the set, spawn `driver.usage(UsageQuery { cwd: project
  root, env: [CLAUDE_CONFIG_DIR of the project's profile] for Claude, [] for agy })` on the
  runtime; when all are back, send an `Input` to the core loop, which emits
  `Event::UsageLoaded(reports)` in `DriverKind::ALL` order. Put it in `src/usage.rs` beside
  the rate-limit code, or a new `src/usage/menu.rs` if that file grows past one subject.
- Tests (with `FakeDriver`, one per kind; `FakeDriver::from_file(..).for_agy()` for agy):
  an agy coordinator launches with the settings folder, MCP server and allowed tools; an agy
  session's id is recorded from `Init` and used on resume; `SwitchDriver` to agy with and
  without handoff ends with a new coordinator on the agy fake, and is refused for a slice;
  `LoadUsage` emits both canned reports; an agy `Models` event leaves Claude's options alone;
  `project_start_slice` with `driver: agy` creates the worktree and starts the agy fake in it.

### C4. TUI: model picker grouped by driver, with the switch confirm

- [ ] Replace `crates/dovetail-tui/src/model_select.rs`'s box with the one `Picker`
  (`src/picker.rs`), as `src/mcp_menu.rs` and the slash-command picker in `src/switcher.rs`
  do. Rows: one per model and effort level, `PickerKey::Model { driver, model, effort }`
  (new variant), `section` = `DriverKind::label()` ("Claude Code", "agy") so the groups show as
  headings while the query is empty. Glyph marks the session's current model and effort. The
  preview shows the model's full name, its efforts, and for the other driver's rows the line
  "Starts a new coordinator session on <driver>".
- [ ] Choosing a row of the session's own driver sends `Command::SetModel` as today.
  Choosing another driver's row (coordinator only; for slices those rows are hidden or shown
  dimmed with "only the coordinator switches driver") opens a confirm: "This starts a new
  session on agy." with **Handoff** (default, Enter) and **Clean start**, Esc cancels. It sends
  `Command::SwitchDriver { handoff }`. Reuse an existing confirm overlay if there is one (grep
  `Overlay::` in `src/app/overlay.rs`); else a small `src/app/confirm.rs`.
- [ ] Keep `/model [name]` (`src/slash.rs`, `Intercept::Model`) and `:model` opening it, with
  the argument typed into the picker's query.
- Tests: rows group by driver in `DriverKind::ALL` order; Enter on an own-driver row gives
  `SetModel`; on the other driver's row gives the confirm, and its keys give
  `SwitchDriver{handoff:true}` / `{handoff:false}` / nothing.

### C5. TUI: the usage menu

- [ ] `DOVETAIL_SLASH` (`src/slash.rs`): add `("usage", "Usage limits of Claude Code and agy",
  "")`; `intercept`: `"/usage" if rest.is_empty() => Intercept::Usage`, with trailing text
  `Refused("/usage")`. Handle the arm in `src/app.rs` next to `Intercept::Mcp`. Note that
  Claude Code has its own `/usage`; Dovetail's replaces it in the chat bar, as `/model` does.
- [ ] `:usage` in `src/command.rs` → a new `Action::ShowUsage` (one entry in the keymap table and
  `:keys`).
- [ ] `src/usage_menu.rs`: the one `Picker` with `preview_from_top()`. It sends
  `Command::LoadUsage` on open and shows "loading" until `Event::UsageLoaded`. Rows: one per
  group, `section` = the driver's label, `detail` = the highest `used_fraction` as a percent.
  Preview: each window's name, a bar or percent, and its reset time (reuse the formatting in
  `src/usage.rs`, which the status row uses), the group's models, the plan and the note.
  A failed report shows one row with the reason. `Tab` reloads, as in the MCP menu.
- Tests: intercept cases in `slash.rs`; the menu's rows and preview from a canned
  `Vec<DriverUsage>`; a failed report shows its reason.

### C6. Docs and close

- [ ] `crates/dovetail-driver/ARCHITECTURE.md` (already current through C1), the core's module
  docs, `docs/architecture.md` if it names a single driver, `GLOSSARY.md` (driver, driver
  switch, quota group), and `config/roles/coordinator.md` if the coordinator needs to know it
  can start slices on agy (keep it under ~20 lines).
- [ ] Manual check with `scripts/dev.sh` in a scratch project: `:usage` lists Claude Code and
  agy groups with percentages and reset times; `/model` shows both groups; picking an agy model
  on the coordinator asks, and Enter starts a new coordinator on agy seeded with the handoff;
  it answers a message and can call `core_report_status`; `:usage` still works.
- [ ] Report `done` to the coordinator with the commits, what was reused, and the gaps.

## Risks and open points

1. **agy's `Init` is once per session, Claude's once per turn.** Any core code that treats
   `Init` as "a turn started" must check the driver (see C3).
2. **One model catalog today.** Without the per-driver catalog, the first agy session would
   replace Claude's model list.
3. **Permission requests on agy** depend on the settings folder; a wrong path means agy
   silently follows the user's `always-proceed`. The C3 launch test must assert the folder and
   its `settings.json`.
4. **Usage reads spawn processes** (`claude` ~1–2 s, `agy` ~1 s, each a few hundred MB
   briefly). Only on `LoadUsage`; never on a timer.
5. **agy's model list before any agy session runs** is empty unless filled from `agy models`.
   The picker then shows no agy rows; acceptable for a first version, say so in the preview.
6. **Rule 1 (subagent routing) was not probed for agy**: every agy event is the session's own.
7. **Messages to agy slices** stay out (#53): an agy slice can report with
   `coordination_report`, but the coordinator cannot `SendMessage` it.
