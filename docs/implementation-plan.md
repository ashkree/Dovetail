# Dovetail implementation plan

**Context.** The design phase is finished and the repo holds only a placeholder binary. This plan sets the order of work for v1: spikes first, then a TUI usable on a mock core, then the shortest route to building Dovetail inside Dovetail.

**Status:** approved; M1–M4 done; waves 1 and 2 (M5–M9) in progress (2026-11-05). Sources: `AGENTS.md`, `docs/design-digest.md`, `docs/design.md`, `docs/ui-session-brief.md`, `docs/screens/`, `README.md`. Checked against this machine on 2026-11-01: Claude Code 2.1.316, cgroup v2 with the `memory` controller delegated to the user session, 30 GB RAM, `claude.ai` login on a Pro plan.

## At a glance

| Stage | Milestones | Result |
| --- | --- | --- |
| Spike gate | S0–S4 | Allowed to proceed, driver chosen, real fixtures captured, three protocol unknowns answered |
| Usable on mock | M1–M4 | The full daily UI runs against `MockCore` with no Claude Code |
| Real, persistent | M5 | A working task tracker on SQLite, with `dovetail add` |
| Live sessions | M6–M9 | Coordinator chat, inbox approvals, slices in worktrees, `ask_user`, `report_status`, `create_task` |
| **Self-hosting point** | **M10** | Restart survival, a stable install and a rehearsal. The rest of the build runs inside Dovetail |
| Remaining v1 | M11–M16 | Subagents, to-dos, board, worktree view, caps, context fill, take-over, maps, hardening |

**Changes from the suggested order in `AGENTS.md`, with reasons**

1. **Board and worktree view move after self-hosting.** They aren't in the self-hosting minimum, and they split cleanly into parallel slices.
2. **Restart survival gets its own milestone (M10).** It's in the minimum and has the riskiest failure modes when Dovetail is running on itself.
3. **The driver, messaging and MCP spikes run before M1.** Real fixtures shape the API, so they should exist before the API is written.
4. **The merman spike moves to just before maps.** Only maps depend on it.

---

## 1. Spike plan

Run in this order. Each writes `docs/spikes/<name>.md`. Code lives in `spikes/<name>/`, outside the workspace. Real sessions run only in scratch repos under `/tmp`, with tiny prompts. S0 and S1 may use the `research` skill.

| ID | Spike | Who runs it | Gates | When |
| --- | --- | --- | --- | --- |
| S0 | Terms of use | **You** (only you can decide) | Everything | First |
| S1 | Driver crate + fixtures | Agent, after S0 | M1 event shapes, M6 onward | Before M1 |
| S2 | Inbound messaging | Agent, after S0 | **Self-hosting** (M8), message timeline | Before M1 |
| S3 | MCP behaviour | Agent, after S0 | M9 `ask_user`, subagent tools | Before M1 |
| S4 | cgroup enforcement | **You** (script supplied) | M13 | Before M13; safe to run any time |
| S5 | merman | Agent measures; **you judge the look in kitty** | M15 | Before M15; machine checks passed (PR #33), look in kitty still yours |
| S6 | `ask_user` role prompt | Agent runs; **you review the result** | Role wording | During M9 |

### S0. Terms of use (you)

**Gates:** everything. If this fails, stop and rethink.

**Procedure**
1. Read the current Claude Code legal and compliance documentation, and the terms for your plan.
2. Check specifically: automated headless use (`claude -p`) through a personal app on a `claude.ai` subscription login, many concurrent sessions, and any difference between subscription and API-key auth.
3. Record the outcome in `docs/spikes/terms-of-use.md`: date, pages read, auth method, your decision.

**Pass:** you are satisfied it's allowed under your auth setup. **Fail:** either move to API-key billing (a cost decision) or rethink the design. I can gather the pages, but I can't decide this for you.

**Related limit:** you're on Pro. Design-phase runs cost $1.30–$1.60 each in API-equivalent terms, so a coordinator plus several slices may hit plan limits quickly. See risk R1.

### S1. Driver crate and fixtures (agent)

**Gates:** `SessionDriver`, and every fixture-based test. **Candidates:** `claude-agent-sdk-rs` 0.6.4 and `rusty_claw` 0.1.0. Both are on crates.io.

**Procedure.** `spikes/driver/` holds one two-session program per crate, run in a scratch repo:

| Check | Test | Pass |
| --- | --- | --- |
| A Multi-session | Two sessions, different prompts, concurrently | No event crosses sessions; each finishes |
| B Interrupt | Interrupt during `sleep 60` | Turn stops within 5 s; the same session takes another prompt |
| C Concurrent permissions | Manual mode, two pending requests; hold one 60 s, answer the other at once | The second proceeds while the first is held; both finish |
| D Mode switch | auto → acceptEdits → manual over the control protocol | Behaviour changes, with no restart |
| E Flag passthrough | `--settings`, `--mcp-config`, `--append-system-prompt-file`, `--worktree`, `--session-id`, `--name`, `--include-partial-messages`, `--permission-prompt-tool stdio`, env vars | All reach the process |
| F Streaming | Text deltas during a reply | Deltas arrive before the final message |
| G Resume | Exit, resume by id, including inside a worktree | Context kept; returns to the worktree |
| H Tolerance | `system/task_*`, `tool_result_meta`, unknown event types | No crash; raw JSON reachable |
| I Cost | `cargo tree` count, added binary size, app RSS | Recorded |

Also re-confirm on 2.1.316, since the spikes ran on 2.1.312:
- `permissionMode: auto` in init on your Pro plan. Spike 3 notes auto needs a supported model.
- Double `result` with a background subagent.
- `--session-id` sets the id, so Dovetail can choose ids instead of parsing them.
- Whether `result.modelUsage` reports the context window (needed for M13).

`--permission-prompt-tool` no longer appears in `--help`, but the flag is still accepted. Confirm it behaves as Spike 3 recorded, and note the new `--permission-prompts` flag (default `host`).

**Decision rule:** take the first crate passing A–H; on a tie, take fewer dependencies. **Fallback:** a hand-written NDJSON layer, preferred over the SDK sidecar because a Node or Python process costs memory on every session.

**Result (2026-11-02, see `docs/spikes/driver.md`):** neither crate works; **hand-written NDJSON layer chosen** (passed A–H). `--permission-prompt-tool stdio` is still required. Re-checks on 2.1.316: `auto` works on Pro; double `result` confirmed (there can be a third, empty one; each carries `result_index`); `--session-id` sets the id; `modelUsage.contextWindow` present. Deny and "always allow" answers untested: cover in M7. Extra fixtures beyond the seven are also recorded.

**Output:** sanitised fixtures in `crates/dovetail-driver/tests/fixtures/`: `plain_turn`, `partial_stream`, `subagent_background`, `double_result`, `concurrent_permissions`, `todo_tools`, `resume`.

### S2. Inbound messaging (agent)

**Why it matters more than the digest says.** The digest gates only the timeline. But if an idle slice doesn't wake on `SendMessage`, the coordinator can't direct slices, so this also gates self-hosting.

**Procedure.** Scratch repo, sessions A and B, both with `crossSessionInbound: "accept"` and `--name`. A stays idle with stdin open.
1. B calls `ListAgents`, then `SendMessage` to A. Capture both raw streams.
2. Repeat while A is mid-turn.
3. Repeat with `--replay-user-messages`.

| Question | Pass | Fallback |
| --- | --- | --- |
| a. Does A's stream show the inbound message? | Yes, with sender identifiable | Timeline from the sender side only |
| b. Does an idle A take a turn with no stdin input? | Yes | **Dovetail relays:** on a `SendMessage` tool call in the sender's stream, the core writes the text to the target's stdin |
| c. Is it delivered mid-turn? | Delivered, order recorded | Record the behaviour; no action needed |
| d. Does `--name` set the name shown by `ListAgents`? | Yes | Tell the coordinator session names in its prompt |

**Result (2026-11-02, see `docs/spikes/messaging.md`):** a, b and d pass, c is queued with order preserved; **M8 does not need the relay.** An idle slice wakes by itself on an inbound `SendMessage`, and the receiver's `origin` object (sender name, socket, verified PID, `msg_id`, body) is joined to the sender's tool result on `msg_id`. With `--replay-user-messages` the receipt shows as a replayed `user` event at the start of the turn. A message sent mid-turn waits for that turn to end. Notes for M8: `SendMessage` is a deferred tool, so list it in `--allowedTools` and mention it in role prompts; the id in `ListAgents` is not a prefix of the stream `session_id`, so match on the names Dovetail assigns; headless sessions show as interactive and `ListAgents` also lists unrelated sessions, so filter by those names; receivers do not reply unless asked, so the slice prompt says to act on coordinator messages.

### S3. MCP behaviour (agent)

**Gates:** `ask_user` semantics. Long tool holds are not in the digest's spike list, but `blocking: true` depends on them. **Procedure:** `spikes/mcp/` has a toy stdio server with `ping(hold_seconds)`.

| Test | Pass | Fallback |
| --- | --- | --- |
| Hold a call 15 min in a main session | Returns normally, or a documented setting allows it | Bounded hold: return "pending", deliver the answer as a user message |
| Call from a foreground subagent | Works | Route through the parent |
| Call from a background subagent | Works | Route through the parent; remove the tools from any subagent guidance |
| Can the server tell which agent called? | Via `_meta` or by stream correlation | Attribute to the parent session |

**Result (2026-11-02, see `docs/spikes/mcp.md`):** all four pass, no fallback. A 15-minute hold returns normally, so `blocking: true` works. Attribution is by stream correlation: `params._meta` carries `claudecode/toolUseId`, matched to the stream's `parent_tool_use_id`; the two can arrive in either order, so the core parks a call briefly. `dovetail mcp` must answer the `server/discover` probe (sent before `initialize`) and unknown methods without hanging. Subagents reach MCP tools via `ToolSearch` (one extra step). Not tested: holds over 15 min, `MCP_TOOL_TIMEOUT`, whether a held call blocks queued user messages (check in M9).

### S4. cgroup enforcement (you)

`scripts/bootstrap.sh` already passed the launch check, but that only proves a scope starts. It doesn't prove a cap is enforced. I'll supply `spikes/cgroup/run.sh`; run it and paste the output.

| Check | Pass |
| --- | --- |
| A memory eater under `MemoryMax=200M`, `MemorySwapMax=0` | Killed; `memory.events` shows `oom_kill` ≥ 1 |
| A forked child over the cap | Counted against the cap; record whether the parent dies too |
| Piped stdin and stdout through `systemd-run --user --scope` | NDJSON passes unchanged |
| PID and exit status | The child's status is visible to the caller |
| Launch overhead | Under 200 ms |
| Scope cleanup | No scope left after exit |

**Fallback:** `MemoryHigh` throttling or `prlimit`; failing that, ship memory display only and warn.

**Result (2026-11-02, see `docs/spikes/cgroup.md`):** all six checks pass on systemd 262; no fallback. M13 must note: systemd's default `OOMPolicy=stop` SIGTERMs the whole scope after an OOM kill, so the session dies with its tool; `-p OOMPolicy=continue` keeps it. Pass `--collect`, or an OOM-killed scope lingers. `systemd-run` turns `$$` into `$` in arguments.

### S5. merman (agent, plus your eye)

**Procedure.** `spikes/merman/` renders a 30-node flowchart with two nesting levels and one style per node kind to PNG, using merman 0.9.3.

| Measure | Pass |
| --- | --- |
| Full render | ≤ 1 s |
| Re-render with a selection change | ≤ 300 ms, or an overlay image solves it |
| Peak added RSS | ≤ 150 MB |
| Node positions | All 30 recovered from the API or SVG, within 5 px |
| Look in kitty | **Your judgement**, with the nine styles visibly distinct |

**Fallback:** elkjs in QuickJS behind `MapRenderer`. Swimlanes aren't affected, since Dovetail lays those out itself.

**Result (2026-11-10, PR #33, see `docs/spikes/merman.md`):** four machine checks pass with merman 0.9.3. Full render took 233 ms cold and 182 ms warm. Re-render with a new selection took 184 ms at 2039×4096 px and 56–71 ms at pane size. Peak added RSS was 86 MiB at 8.4 MP and 24 MiB at pane size. All 30 positions came from the layout JSON, with the worst error 1.4 px. **The look in kitty is still yours to judge.** M15 must give the selected node one combined class: merman's PNG uses only a node's first class. Weight: 105 new crates, ≈ 14 MB binary, one EPL-2.0 crate.

### S6. `ask_user` role prompt (agent runs, you review)

**Procedure.** During M9, run three trials each of two slices in a scratch repo: one with tool descriptions only, one with the role prompt. Give a task that can't finish without a missing value. **Pass:** the role-prompt slice calls `ask_user` in 3 of 3 runs instead of ending its turn with a question. **Fallback:** stronger wording, or a skill.

---

## 2. Crate layout and dependencies

```
crates/
├── dovetail-api/      # plain data: Snapshot, Event, Command, apply(), the map model (map module); serde only
├── dovetail-driver/   # SessionDriver, ClaudeDriver, FakeDriver, fixtures
├── dovetail-core/     # sessions, storage, worktrees, socket server, MCP tool handlers
├── dovetail-mock/     # MockCore: same API, seeded, time-free in tests
├── dovetail-tui/      # buffers, regions, keymap, theme, render; depends only on dovetail-api
├── dovetail-maps/     # draw_map input, validation and limits, MapRenderer (added at M15); re-exports dovetail_api::map
└── dovetail/          # binary: TUI (default), add, mcp, hook
```

Dependency direction: `api ← driver ← core`, `api ← mock`, `api ← tui`, `api ← maps ← core` (the core validates and stores `draw_map` input; the mock only tests against maps), and `dovetail` wires them together. `dovetail-tui` depends only on `dovetail-api`, so Cargo itself stops the UI reaching past the boundary. The plain map model sits in `dovetail_api::map` for the same reason: map content can travel in the snapshot while merman stays in `dovetail-maps`, out of the TUI.

Workspace changes at M1: add `exclude = ["spikes"]` to the root `Cargo.toml`, since nested packages otherwise fail, and add `spikes/*/target` to `.gitignore`.

| Dependency | Used in | Features | Reason |
| --- | --- | --- | --- |
| `serde`, `serde_json` | api, driver, core, mock, tui, maps, binary | `derive` | Plain-data API, NDJSON, MCP; maps builds the `draw_map` JSON schema; the mock builds tool inputs and `--dump` prints JSON lines; the TUI reads a permission request's tool input (`command`, `file_path`) for the inbox preview |
| `ratatui` 0.30 | tui | default | Required |
| `crossterm` 0.29 | tui | default | Required; no kitty keyboard protocol |
| `nucleo` 0.5 | tui | default | Fuzzy picker (required) |
| `pulldown-cmark` 0.13 | tui | no default features | Markdown (required) |
| `tokio` | driver, core | `rt-multi-thread`, `process`, `io-util`, `net`, `sync`, `time`, `macros` | Process and socket IO, core only |
| `rusqlite` 0.41 | core | `bundled` (FTS5 included) | Required |
| `rusqlite_migration` | core | none | Small migration layer; check it supports this `rusqlite` version |
| `toml` | core | default | Recipe and `config.toml` |
| `thiserror`, `anyhow` | libraries, binary | none | Required by conventions |
| `tracing`, `tracing-subscriber`, `tracing-appender` | core, tui, binary | `fmt`, `env-filter` | File logging only. The TUI uses only the `tracing` macros; the binary sets up the file writer (M2) |
| `jiff` | core, tui, binary | default | Local-time display with no unsafe of ours; the binary passes the system time zone to the UI; the core names journal files by local date (ADR 0012) |
| `unicode-width` | tui | default | Pad, clip and wrap rows by terminal cell width (already a `ratatui` dependency) |
| `uuid` | core | `v4` | Choose session ids |
| `notify` (latest stable, not the 9 rc) | core | `default-features = false` | Watch native task files (M11). Its only default feature is `macos_fsevent` (FSEvents bindings through `objc2`); Linux uses inotify without it |
| *none:* hand-written NDJSON layer on `tokio` + `serde_json` | driver | n/a | Decision 4a fallback. S1 (2026-11-02): `claude-agent-sdk-rs` and `rusty_claw` both fail C (concurrent permissions) and H (tolerance); the hand-written layer passed A–H (18 crates, +1.39 MB, 4.3 MB RSS) |
| `merman` `=0.9.3` | maps | `default-features = false`, `diagram-flowchart`, `layout-elk`, `png` | Decision 23c, S5 passed: renders structure maps to PNG and gives node positions. 105 new crates, ≈ 14 MB, one EPL-2.0 crate (`merman-elk-layered`, notice must ship) |
| `png` | maps, dev only | default | Tests decode the rendered PNG to check the selection ring is drawn (merman already pulls it in) |
| `resvg` `=0.47.0` | maps | `default-features = false`, `text`, `system-fonts` | M15 (f): rasterises the swimlane map's SVG, which Dovetail draws itself (23e). No new crates: merman's PNG export already pulls in this version |
| `rustix` 1 | core | `std`, `process` only | `waitpid` with `WUNTRACED` and `SIGCONT`, so an editor or native `claude` that stops itself (Ctrl-Z) is continued instead of hanging the terminal (task #32); no unsafe of ours, and already in the tree through `crossterm` and `tempfile` |
| `signal-hook` | tui, binary | default | Catch SIGINT, SIGTERM and SIGHUP on a thread so the TUI quits cleanly (terminal restored, sessions ended and reaped) instead of dying; the binary's `dovetail upgrade` sets a flag on SIGINT/SIGTERM so a Ctrl-C stops and reaps cargo (ADR 0014); no unsafe of ours |
| `insta`, `tempfile` | dev only | default | Snapshot tests, temp databases |

**Deliberately not added**
- `clap`: four subcommands are parsed by hand.
- `ratatui-image` (planned for M15): kitty graphics are a few hand-written escape sequences (`dovetail-tui` `canvas::kitty`), and the canvas needs kitty's source rectangle for pan and zoom, so the crate would add weight without doing the work.
- `rmcp`: `dovetail mcp` is a small synchronous JSON-RPC loop using only the standard library and `serde_json`. One bridge runs per session, so its memory matters, and three tools don't justify a framework.
- `dirs`: XDG paths are ten lines.
- `rand`: `MockCore` uses a small seeded generator.
- `async-trait`: the driver's methods are synchronous enqueues.

---

## 3. Core API sketch

The UI thread has no tokio. It blocks on one `std::sync::mpsc` channel fed by a terminal input thread and a core forwarder, using `recv_timeout` only when a real deadline is pending (a key-sequence timeout, or the 33 ms coalescing window). No idle ticks.

```rust
// crates/dovetail-api/src/lib.rs (fields abbreviated; every public type gets a doc comment)
pub const PROTOCOL_VERSION: u32 = 1;

pub struct Snapshot {
    pub seq: u64,
    pub now_ms: i64,
    pub project: Option<ProjectSummary>,
    pub projects: Vec<ProjectSummary>,
    pub tasks: Vec<Task>,
    pub agents: Vec<Agent>,
    pub todos: Vec<TodoItem>,
    pub inbox: Vec<InboxItem>,
    pub activity: Vec<ActivityItem>, // bounded tail
    pub messages: Vec<AgentMessage>, // bounded tail
    pub conversations: HashMap<AgentId, ConversationTail>, // tails only
    pub totals: Totals,              // memory, needs-you count
    pub warnings: Vec<Warning>,      // status-bar warnings
    pub maps: Vec<MapSummary>,       // M15: the map shelf, newest first; content via LoadMap
}

pub struct Task {
    pub id: TaskId, pub title: String, pub status: TaskStatus,
    pub p_level: Option<PLevel>, pub bucket: Option<Bucket>, pub order: i64,
    pub deps: Vec<TaskId>, pub notes: String, pub pr: Option<u32>,
    pub follow_up_of: Option<TaskId>, pub coordinator_created: bool,
}

pub struct Agent {
    pub id: AgentId, pub name: String,
    pub kind: AgentKind,             // Coordinator | Slice | Subagent { background: bool }
    pub parent: Option<AgentId>, pub task: Option<TaskId>,
    pub worktree: Option<WorktreeRef>,
    pub status: AgentStatus,         // SettingUp | Working | Idle | NeedsYou | Done | Error
    pub status_line: String,
    pub memory_mb: Option<u32>, pub cost_usd: f64,
    pub permission_mode: PermissionMode,
    pub context_pct: Option<u8>,
    pub progress: Option<SubagentProgress>, // background subagents only
    pub last_activity_ms: i64,
    pub lock: SessionLock,           // Headless | TakenOver
}

pub enum InboxKind {
    Permission { tool: String, input: serde_json::Value, reason: Option<String>,
                 suggestions: Vec<RuleSuggestion> },
    Question { text: String },   // never blocking (ADR 0007)
    FinishedTurn { preview: String },
    Error { text: String },
}

pub enum Event {
    TaskUpserted(Task), TaskRemoved(TaskId),
    AgentUpserted(Agent), AgentRemoved(AgentId),
    TodosReplaced { list: TodoListId, items: Vec<TodoItem> },
    InboxUpserted(InboxItem), InboxResolved { id: InboxId },
    ActivityAdded(ActivityItem), MessageAdded(AgentMessage),
    ConversationAppended { agent: AgentId, entry: ConversationEntry },
    AssistantDelta { agent: AgentId, entry: EntryId, text: String },
    ConversationPage { agent: AgentId, entries: Vec<ConversationEntry>, has_more: bool },
    TotalsChanged(Totals), WarningSet(Warning), WarningCleared(WarningId),
    MapUpserted(MapSummary), MapLoaded(Map), // M15
    CommandFailed { message: String },
}

pub enum Command {
    Prompt { agent: AgentId, text: String },
    Interrupt { agent: AgentId },
    AnswerPermission { item: InboxId, answer: PermissionAnswer },
    AnswerQuestion { item: InboxId, text: String },
    DismissInbox { item: InboxId },
    CyclePermissionMode { agent: AgentId },
    NewSlice { task: Option<TaskId>, name: String, brief: String },
    CloseSlice { agent: AgentId },
    TakeOver { agent: AgentId },
    CreateTask(NewTask), UpdateTask { id: TaskId, patch: TaskPatch },
    ArchiveTask { id: TaskId }, ReorderTask { id: TaskId, delta: i32 },
    RemoveWorktree { agent: AgentId, confirmed: bool },
    OpenProject { id: ProjectId }, RegisterProject { path: PathBuf },
    LoadConversation { agent: AgentId, before: Option<EntryId> },
    LoadMap { id: MapId, version: Option<u32> }, // M15: answered by Event::MapLoaded(Map)
    Resync, Quit { end_sessions: bool },
}

pub enum ServerMsg { Hello { protocol: u32 }, Snapshot(Snapshot), Event { seq: u64, event: Event } }

/// Pure: the only way client state changes. Shared by the UI, MockCore and tests.
pub fn apply(state: &mut Snapshot, seq: u64, event: Event) -> ApplyResult; // Applied | Gap
```

**How it survives the daemon split**
- All types are plain serde data with no channels, handles or `Arc`.
- v1 sends `ServerMsg` over in-process channels. A daemon sends the same values as NDJSON over a unix socket, with only the transport changing.
- Every event carries `seq`. On `Gap`, the client sends `Resync` and receives a new `Snapshot`, so a reconnect needs no new logic.
- Snapshots carry conversation tails only. Older history comes through `LoadConversation`, so a remote client stays cheap.
- `Action` (keys) is UI-local. Actions become `Command`s or local view changes, and no key logic crosses the boundary.

`MockCore` implements the same `CoreLink` as the real core and is selected with `dovetail --mock`, kept permanently for demos and snapshot tests. With a fixed seed and an injected clock, its output is deterministic.

---

## 4. `SessionDriver`, fake driver and event mapping

```rust
// crates/dovetail-driver/src/lib.rs
pub trait SessionDriver: Send + Sync + 'static {
    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError>;
}

/// Methods only enqueue, so a held permission can never block anything else.
pub trait Session: Send {
    fn send(&self, input: UserInput) -> Result<(), DriverError>;
    fn interrupt(&self) -> Result<(), DriverError>;
    fn set_permission_mode(&self, mode: PermissionMode) -> Result<(), DriverError>;
    fn answer_control(&self, id: ControlId, answer: ControlAnswer) -> Result<(), DriverError>;
    fn take_events(&mut self) -> tokio::sync::mpsc::UnboundedReceiver<DriverEvent>;
    fn shutdown(&self, grace: Duration);
}

pub struct SessionSpec {
    pub session_id: Uuid, pub resume: bool, pub name: String, pub cwd: PathBuf,
    pub worktree: Option<String>, pub settings_path: PathBuf, pub mcp_config_path: PathBuf,
    pub role_prompt_path: PathBuf, pub permission_mode: PermissionMode,
    pub task_list_id: String, pub memory_max_mb: Option<u32>, pub model: Option<String>,
}
```

**Launch flags (`SessionSpec` → argv)**

`claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --replay-user-messages --permission-prompt-tool stdio --permission-mode auto --session-id <uuid> --name <name> --settings <file> --mcp-config <file> --append-system-prompt-file <role> [--worktree <slice>]`

Environment: `CLAUDE_CODE_ENABLE_TODO_TOOLS=1`, `CLAUDE_CODE_TASK_LIST_ID=<list id>`. M13 prefixes the command with `systemd-run --user --scope`.

Generated `--settings` and `--mcp-config` files go in `~/.local/state/dovetail/run/<agent-id>/`, never in a repo.

**`FakeDriver`** replays a fixture with optional scripted delays, checks the commands it receives, and can pause at a marker so tests can interleave answers. It covers subagent routing, double `result` and concurrent permissions with no network.

| Stream input | `DriverEvent` | Core → API |
| --- | --- | --- |
| `system/init` | `Init { mode, model, session_id }` | Agent update; `Warning` if not `auto`, then run as manual |
| `stream_event` text delta | `TextDelta` | `AssistantDelta` (coalesced ≤ 30 fps) |
| `assistant` text | `AssistantText` | `ConversationAppended`, stored |
| `assistant` tool use | `ToolUse` | Folded tool entry; `SendMessage` also gives `MessageAdded` |
| Any event with `parent_tool_use_id` | Same event, tagged `parent` | Routed to that subagent's node |
| `system/task_*`, `background_tasks_changed` | `SubagentTask` | Agent upsert: progress, usage, summary, output file |
| `control_request` `can_use_tool` | `PermissionRequest` | `InboxUpserted(Permission)`, `agent_id` attributes it |
| `system/permission_check_status` | Ignored | Nothing |
| `result` | `TurnResult` | Usage and cost; a `FinishedTurn` only if no background tasks remain |
| Process exit | `Exited { status }` | `Done`, or `Error` inbox item; OOM detected at M13 |
| Unknown type | `Unknown(raw)` | Logged, never fatal |

Session state is an explicit machine in `dovetail-core/src/session/state.rs`. It's done only when the process exits or all background tasks have reported, never on the first `result`.

---

## 5. Storage schema

`registry.db` plus a `state.db` per project. Each is served by the single DB thread through typed requests. WAL on. Migrations are forward-only and start from empty.

```sql
-- registry.db (as of registry migration 0003: a git dir is optional, ADR 0005, and a research
-- project has neither repo nor git dir, decided 2026-11-11)
CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'code' CHECK (kind IN ('code', 'research')),
  repo_path TEXT, git_common_dir TEXT UNIQUE, last_opened_ms INTEGER,
  CHECK (kind = 'research' OR repo_path IS NOT NULL),
  CHECK (kind = 'code' OR (repo_path IS NULL AND git_common_dir IS NULL)));

-- state.db
CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL,
  p_level INTEGER, bucket TEXT, sort_order INTEGER NOT NULL DEFAULT 0, notes TEXT NOT NULL DEFAULT '',
  pr INTEGER, follow_up_of INTEGER REFERENCES tasks(id),
  coordinator_created INTEGER NOT NULL DEFAULT 0, created_ms INTEGER, updated_ms INTEGER);
CREATE TABLE task_deps (task_id INTEGER, depends_on INTEGER, PRIMARY KEY(task_id, depends_on));

CREATE TABLE agents (id TEXT PRIMARY KEY, kind TEXT, name TEXT, parent_id TEXT,
  task_id INTEGER REFERENCES tasks(id), session_id TEXT, transcript_path TEXT, task_list_id TEXT,
  worktree_path TEXT, branch TEXT, setup_state TEXT, status TEXT, status_line TEXT,
  permission_mode TEXT, cost_usd REAL DEFAULT 0, tool_use_id TEXT, output_file TEXT,
  created_ms INTEGER, last_activity_ms INTEGER, closed_ms INTEGER);

CREATE TABLE todo_items (list_id TEXT, item_id TEXT, agent_id TEXT, PRIMARY KEY(list_id, item_id));

CREATE TABLE inbox_items (id INTEGER PRIMARY KEY, agent_id TEXT, kind TEXT, payload TEXT,
  blocking INTEGER DEFAULT 0, created_ms INTEGER, resolved_ms INTEGER, answer TEXT);

CREATE TABLE activity (id INTEGER PRIMARY KEY, at_ms INTEGER, kind TEXT, agent_id TEXT, text TEXT);

CREATE TABLE messages (id INTEGER PRIMARY KEY, at_ms INTEGER, from_agent TEXT, to_agent TEXT,
  text TEXT, direction TEXT);

-- Conversation text only, never tool output (Decision 20); tool activity stored as summaries
CREATE TABLE conversation_entries (id INTEGER PRIMARY KEY, agent_id TEXT, turn INTEGER,
  at_ms INTEGER, kind TEXT, text TEXT);
CREATE INDEX conversation_by_agent ON conversation_entries(agent_id, id);

-- Added at M5 with the tasks (decided 2026-11-03, ADR 0003)
CREATE TABLE task_related (a INTEGER REFERENCES tasks(id), b INTEGER REFERENCES tasks(id),
  PRIMARY KEY(a, b), CHECK (a < b));                    -- one row per pair; both ways in the API
CREATE TABLE decisions (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id),
  at_ms INTEGER NOT NULL, author TEXT NOT NULL,          -- 'user' or an agent id
  text TEXT NOT NULL, why TEXT, alternatives TEXT NOT NULL DEFAULT '[]', -- JSON [{option, why_not}]
  supersedes INTEGER REFERENCES decisions(id));

-- Added at M15 (task #35): the shelf row describes the latest version; `model` is the
-- validated `MapContent` as JSON. A map keeps its id and type across versions.
CREATE TABLE maps (id TEXT PRIMARY KEY, task_id INTEGER REFERENCES tasks(id), title TEXT NOT NULL,
  type TEXT NOT NULL, current_version INTEGER NOT NULL, drawn_by TEXT,
  created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
CREATE TABLE map_versions (map_id TEXT NOT NULL REFERENCES maps(id), version INTEGER NOT NULL,
  model TEXT NOT NULL, drawn_by TEXT, created_ms INTEGER NOT NULL, PRIMARY KEY(map_id, version));
```

**Registry migrations** (`db/migrations/registry/`)
- M5 `0001_init`: `projects`
- Wave 2 `0002_optional_git`: `git_common_dir` nullable, so any folder can be a project (ADR 0005)
- Research-only projects `0003_project_kind`: `projects.kind` (`code` or `research`, existing rows become `code`), `repo_path` nullable; a research project has neither path, `UNIQUE` still holds for the git dirs that are set (decided 2026-11-11, ADR 0015's open point)

**Migrations** (`state.db`, `db/migrations/state/`)
- M5 `0001_init`: `tasks`, `task_deps`, `task_related`, `decisions`
- M6 `0002_agents_conversation`
- M7 `0003_inbox_activity`
- Wave 2 `0004_slice_queue` (queued slice starts survive a restart, decided 2026-11-06)
- M10 `0005_restart_state` (held coordinator notes, kept unowned worktrees)
- Task #1 `0006_entry_attachments` (image paths on user entries)
- M11 `0007_todos` (`todo_items`)
- M11 `0008_subagent_progress` (`agents.progress`, a subagent's progress as JSON)
- M11 `0009_subagent_tokens` (`agents.subagent_tokens`, a session's total summed from task events, task #17)
- Task #27 `0010_agent_claude_home` (`agents.claude_home`, the Claude config folder a session ran under; ADR 0009)
- M14 `0011_agent_takeover` (`agents.takeover`, a session's take-over lock as JSON, task #32)
- M15 `0012_maps`
- R1 (M17 in `docs/plan/research-sessions.md`) `0013_research`: `research_rounds`, `findings`, `sources`, `finding_sources`, `reports` (written from R2), `tasks.research_kind` and `tasks.altitude` (`docs/research.md` §3). R2's `decision_evidence` is `0014`.
- R2 (M18) `0014_decision_evidence`: `decision_evidence (decision_id, kind, ref_id)`, the findings and sources an accepted `research_propose` decision rests on (ADR 0015); `decisions` itself is unchanged (ADR 0003).

Tables arrive with the milestone that uses them, so no unused tables sit around.

**Left for v1.1**
- `claims (pr, agent_id, ...)`
- Moving `tasks.notes` into `tasks/<id>.md` (Decision 19c); the decision log itself is in v1 (ADR 0003)
- An FTS5 `search_index`, backfilled from `conversation_entries`, `inbox_items`, `messages` and notes
- `journal_days`
- Map comments

**Why `conversation_entries` is in v1.** Restoring a conversation needs its text. Decision 20 already allows storing conversation text, and this avoids depending on the undocumented transcript format. `transcript_path` is still recorded for take-over and deep search.

---

## 6. Milestones

Every milestone ends with `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` passing, plus a "how to try it" summary. Then I stop and wait for you.

**Review step.** Each wave ends with `/code-review` from the latest review tag, followed by `/retro`. Tags are `pre-m1`, set before M1 starts, then `review-1`, `review-2` and so on, each set when that review closes.

**TDD.** M1 used the `tdd` skill. From M5 on, tests are written when something breaks (decided 2026-11-05).

### Waves (decided 2026-11-05)

From M5, milestones are built in waves, gated once per wave (Opus `code-review`, then your `/retro`), and merged to `main` by the agent.

- **Contracts commit** (on `wave-1`, before any track): `dovetail_api::socket` (NDJSON request and response types for `dovetail add` and the MCP bridges), `dovetail_core::{paths, recipe}`, empty core modules, and the `add`/`mcp`/`hook` dispatch in `main.rs`. `rusqlite_migration` is dropped: its latest release needs rusqlite 0.40, so migrations use SQLite's `user_version` directly.
- **Wave 1: four parallel tracks, written by Sonnet agents in their own worktrees, `cargo build` only, no new tests.**
  - **A. Storage and tracker:** all of M5 (DB thread, migrations, projects, `config.toml` with `max_slices`, socket server, `dovetail add`, the real core's `CoreLink` for tasks, bootstrap).
  - **B. Driver:** the `dovetail-driver` half of M6 (`SessionDriver`, `ClaudeDriver`, `FakeDriver`, event mapping).
  - **C. Worktrees:** the lower half of M8 (`dovetail hook worktree-create`, `core::worktree` create, setup, inspect, remove).
  - **D. MCP bridge:** the lower half of M9 (`dovetail mcp`, tool schemas including `start_slice`, role-prompt edits).
- **Wave 2: one Opus agent, in order:** the session core of M6, M7, the slice flow of M8, the tool handlers of M9 including `start_slice` ([ADR 0004](adr/0004-start-slice-tool.md)). Also [ADR 0005](adr/0005-explicit-projects-any-folder.md): no silent registration, `:project add` and `add_project`, in-memory state until registered, and non-git projects whose slices share the folder.
- **Live checks:** one batch after wave 2, in `scripts/live-checks.sh`, which you run. Scratch repos in `/tmp`, tiny prompts, Haiku except where auto mode needs a supported model.
- **Then M10**, the self-hosting point.
- **Rules from the wave-1 retro:**
  - Every value that crosses a track boundary (wire types, tool inputs, env var names, ids, flags) is defined in the contracts commit, in `dovetail-api` where both sides can import it.
  - A track agent reports each "done when" item as met or unverified; only the integrator ticks boxes, after checking.
- **Rules from the wave-2 retro:**
  - The gate is `scripts/check.sh`; an agent runs it before reporting done.
  - Commit at every milestone and every coherent step inside one (WIP commits are fine), one milestone per commit even when files overlap (`git add -p`).
  - The final report lists every place the code departs from a spec line, as a question for the user.

### M1. Workspace, API, mock core
- **Spikes needed:** S1–S3 (event shapes)
- **Scope:** create all crates except maps. `dovetail-api` types and `apply()`. `MockCore` generating status changes, streaming markdown replies, inbox items and subagents. `dovetail --mock --dump` prints events, with no UI yet.
- **Files:** root `Cargo.toml`, `.gitignore`, `crates/dovetail-api/`, `crates/dovetail-mock/`, `crates/dovetail/src/main.rs`, README status and layout sections
- **Done when:**
  - [x] Workspace builds with all crates
  - [x] `dovetail --mock --dump` shows a believable, seeded event stream
  - [x] `dovetail-tui` has no dependency on tokio or core
- **Tests:** `apply()` for every event, gap detection, serde round-trip, mock determinism

### M2. TUI shell: layout, keymap, tree, inbox
- **Scope:** terminal setup and panic hook; event loop with dirty flag; Layout B; `Region` and `Buffer` abstractions; Normal mode; keymap table with sequences and timeouts; `Action` enum; theme struct; status bar. Task tree with grouping (Unsorted, Now, Next, Later, Done), P-levels, mismatch marker and tree keys. Inbox with preview and keys `y A d D r x Enter`.
- **Files:** `crates/dovetail-tui/src/{app,event_loop,keymap,action,theme,region}.rs`, `buffers/{tree,inbox}.rs`, `crates/dovetail/src/main.rs`
- **Done when:**
  - [x] `dovetail --mock` opens Layout B with live updates and quits cleanly with `:q` (pty run: alternate screen entered and left, exit 0)
  - [x] Terminal is restored on exit and on panic (pty run: `:panic`, a debug-build command, leaves the alternate screen and shows the cursor)
  - [x] CPU is idle when nothing changes (pty run: 0.00 % CPU and 0 bytes drawn over 8 s while the mock was quiet; `next_wake` returns `None` when clean)
  - [x] Inbox actions change state through `Command`s
- **Tests:** mode machine, keymap (`gg`, `dd`, `]]`, `<leader>ss`, timeouts, ambiguity), tree grouping and sorting, snapshot of Layout B

### M3. Conversation and reading
- **Scope:** conversation buffer with the readability rules built in from the start: 120-column centred column, spacing, styled and coloured markdown, off-white body, folded tool lines, hidden thinking, truncation with "show more", `j k gg G ]] [[ zc zo`, solid-background toggle. Insert mode with multiline input. Agent switching, `<leader>c`, `<leader><Tab>`.
- **Files:** `buffers/conversation.rs`, `render/{markdown,reading_column}.rs`, `input.rs`
- **Done when:**
  - [x] ~~You can read a streamed mock reply comfortably in kitty and approve the theme~~ (waived by you, 2026-11-05)
  - [x] Streaming redraws stay at or below 30 fps (pty run with the mock at 200x: 28.8 frames a second, logged by the loop at trace level)
  - [x] `Enter`, `Ctrl-j`, `Alt-Enter`, `Ctrl-w`, `Ctrl-u` work (behaviour tests drive them through `App`)
- **Tests:** markdown spans, spacing, wrapping, snapshots at 80, 120 and 200 columns

### M4. Command mode, picker, switcher, quick capture
- **Scope:** Command mode (`:q`, `:spawn`, `:interrupt`, plus aliases for actions). One picker component using `nucleo`, used for `<leader>ss` and `<leader><leader>`. Quick capture with its token parser, link display and `Ctrl-l`. Task detail buffer with fields, notes and agents; the `e` notes handoff to `$EDITOR` lands in M5 with real storage (A4). Permission-mode cycle and badge. Context-fill display from mock values. In-buffer `/ n N`. Activity and message timeline buffers.
- **Files:** `command.rs`, `picker.rs`, `capture.rs`, `buffers/{task_detail,activity,messages}.rs`
- **Done when:**
  - [x] **The TUI is usable on the mock with no Claude Code**: every self-hosting flow can be walked by keyboard (chat, approve, answer, capture, start a slice, switch, interrupt, cycle mode, read the feeds; 55 tests in `tests/m4.rs` plus a pty walk of the real binary)
  - [x] `<leader>ss` opens instantly with preview, and needs-you agents sort first (a test opens it over 100 agents and draws in well under 500 ms in a debug build)
- **Tests:** capture parser, picker ordering, permission cycle, snapshots of overlays

### M5. Storage, projects and a real task tracker
- **Scope:** DB thread, typed requests, migrations. `dovetail-core` with `CoreLink`. Project identity from `git rev-parse --path-format=absolute --git-common-dir`. Project registration. Recipe reader for `~/.config/dovetail/projects/<id>.toml`. `config.toml`. Unix socket at `$XDG_RUNTIME_DIR/dovetail/<project-id>.sock`. `dovetail add`.
- **Files:** `crates/dovetail-core/src/{lib,db/,projects,recipe,config,paths,socket}.rs`, `crates/dovetail/src/{main,add}.rs`, `config/projects/dovetail.toml`, `scripts/bootstrap.sh`
- **Done when:**
  - [x] `dovetail` in a registered repo shows persistent tasks with no sessions
  - [x] Tasks survive restart
  - [x] `dovetail add "x !p1 @now"` appears live if the app is running, or on next open if not
  - [x] Recipe format is final and bootstrap matches it
- **Tests:** migrations from empty, typed requests, project identity from a worktree, recipe parsing, `add` both ways (deferred: tests are written when something breaks, decided 2026-11-05)

### M6. Driver and one coordinator session
- **Spikes needed:** S0, S1
- **Scope:** `ClaudeDriver`, `FakeDriver`, the session state machine, launch settings, coordinator in the main checkout, streaming chat, interrupt, finished-turn inbox items, stored conversation entries, `<leader>sx`, messaging-blocker warnings.
- **Files:** `crates/dovetail-driver/src/{lib,claude,fake,events}.rs`, `crates/dovetail-core/src/session/{mod,state,launch}.rs`
- **Done when:**
  - [ ] Real streaming chat with a coordinator in a scratch repo
  - [ ] Interrupt works
  - [ ] Non-`auto` init shows a warning
  - [ ] Child processes are always reaped
- **Tests:** fixtures for plain turn, double `result`, partial stream; live test behind `DOVETAIL_LIVE_TESTS=1` (deferred: tests are written when something breaks, decided 2026-11-05)

### M7. Live permissions
- **Scope:** `can_use_tool` to inbox and back, concurrent answers, `y A d D`, rule suggestions for always-allow, `<leader>sm` over the control protocol, `permission_denials` surfaced, activity feed from real events (slice lifecycle and messages; the rest moved to M16, see below).
- **Decided 2026-11-06:** "always allow" (`A`) applies the offered rule to the session only. Claude Code offers rules with destination `localSettings` or `projectSettings`, which would write into the repo; Dovetail rewrites the destination to `session` before sending the answer.
- **Moved to M16 (decided 2026-11-06):** activity items for permission answers, coordinator start and exit, task changes, and PR and CI events.
- **Files:** `crates/dovetail-core/src/{inbox,permissions}.rs`
- **Done when:**
  - [ ] In manual mode, a real request is answered from the inbox
  - [ ] One held request never blocks another
  - [ ] Mode changes with no restart, and the badge follows
- **Tests:** `concurrent_permissions` fixture, always-allow payloads, mode mapping (`manual`) (deferred: tests are written when something breaks, decided 2026-11-05)

### M8. Slices, worktree hooks, cleanup
- **Spikes needed:** S2 (done: no relay needed)
- **Scope:** `dovetail hook worktree-create` (idempotent; symlinks only; recipe setup; prints the path). `SettingUp` state in the tree. `<leader>sn` and `:spawn`. Slice brief from task plus your text. First prompt sent after init. *As built (wave 2): the first prompt is sent when the `WorktreeCreate` hook records its outcome (`hook.json`), which stands in for "after init": whether `system/init` arrives before the first input is unknown, and waiting for it could deadlock; the outcome also tells success from failure, so a failed setup never gets a prompt.* Worktree removal: check for uncommitted or unpushed work, ask, remove our files, `git worktree remove`, delete the `dovetail/*` branch. Message timeline from `SendMessage` calls and `origin` objects joined on `msg_id` (no relay, per S2); sessions launch with `--replay-user-messages` and `SendMessage` in `--allowedTools`. Archive flow with `dd`.
- **Files:** `crates/dovetail-core/src/{worktree,slice,messaging}.rs`, `crates/dovetail/src/hook.rs`
- **Done when:**
  - [ ] A real slice starts in a hook-created worktree outside the repo
  - [ ] Failed setup means no first prompt, and an inbox error
  - [ ] A setup longer than Claude Code's default hook timeout finishes under the 15-minute timeout (B5)
  - [ ] In a scratch repo, `gh pr merge` in auto mode raises a Permission request through the launch-settings ask rule, or the B7 fallback is recorded
  - [ ] Coordinator and slice exchange messages in a scratch repo
  - [ ] Archiving removes everything, after asking when work is unpushed
  - [ ] Only branches appear in the repo
- **Tests:** hook idempotence, cleanup against real temp git repos, dirty and unpushed detection (deferred: tests are written when something breaks, decided 2026-11-05)

### M9. MCP bridge, `ask_user`, `report_status`, `create_task`, role prompts
- **Spikes needed:** S3; S6 runs here
- **Scope:** `dovetail mcp` as a synchronous stdio server, forwarding to the app socket with its agent id. Tool handlers in core. Question inbox items, after permission requests (ADR 0007: no blocked flag). `r` reply. `create_task` for the coordinator only (ADR 0002). `log_decision` for the coordinator and slices (ADR 0003): schema `{task?, decision, why?, alternatives?: [{option, why_not?}], supersedes?}`; slices default to their task, the coordinator must name one. Role prompts passed at launch.
- **Role prompt fix:** `config/roles/coordinator.md` tells the coordinator to call `draw_map`, which won't exist until M15. Remove that line now and restore it at M15. Also tell the coordinator that the user starts slices and it recommends them (B1), and that it merges only on the user's yes and then pulls `main` (B7). Tell slices to push and open a PR once checks pass (B7). Then run `scripts/bootstrap.sh --force`, because your installed copy from the earlier run has the old text.
- **Files:** `crates/dovetail/src/mcp.rs`, `crates/dovetail-core/src/tools/{mod,ask_user,report_status,create_task}.rs`, `config/roles/*.md`
- **Done when:**
  - [ ] An `ask_user` reaches the inbox and the chat, returns at once, and your reply arrives as a message (ADR 0007; was: a blocking call your reply unblocks)
  - [ ] Status lines appear in the tree
  - [ ] A coordinator `create_task` lands in Unsorted, marked until seen; a slice can't call it
  - [ ] App exit gives the agent a clear error, not a hang
  - [ ] S6 recorded
- **Tests:** MCP protocol, tool schemas, socket forwarding, `ask_user` returning at once with the answer arriving as a message (ADR 0007) (deferred: tests are written when something breaks, decided 2026-11-05)

### M10. Resume, stable install, rehearsal — **SELF-HOSTING POINT**
- **Scope:** restore on open (coordinator immediately, slices on first touch). Pending inbox items reconciled after restart. A stale-state sweep for agents and worktrees. Stable install (`cargo install --path crates/dovetail --root ~/.local`), with hook and MCP paths using the installed path, not `target/`. `docs/self-hosting.md` covering the daily flow, upgrades and recovery.
- **Decided 2026-11-06:**
  1. **Cut-off turns.** A turn cut off by quit, kill or crash raises a Needs-you item on reopen, "<agent>'s turn was cut off by a restart", with the last user prompt as its preview. The agent is never continued automatically.
  2. **In-flight items.** Pending permission requests leave Needs you at restart and become one Activity note each ("Write a.txt: request expired at restart"). Pending `ask_user` questions stay in Needs you; answering one sends the agent a normal user message, "Answer to your question '<q>': <answer>", resuming it if needed.
  3. **Upgrades.** `scripts/install.sh` runs `cargo install --path crates/dovetail --root ~/.local` and refuses while a Dovetail app runs ("close Dovetail first"). Hook and MCP config always use the running binary's path (`std::env::current_exe`), so the installed app never points at `target/`.
  4. **Stale-state sweep on open.** An agent whose worktree directory is gone is archived with an Activity note. A worktree under Dovetail's worktrees directory that no agent owns raises one "Remove worktree" item (branch, unpushed commits, uncommitted changes); `y` removes it, `d` keeps it and records that, so it is not asked again. Nothing is deleted without `y`.
- **Done when:**
  - [x] **Rehearsal in a scratch repo:** from the installed binary, chat with the coordinator, start two slices, answer one permission and one `ask_user`, have a slice commit, quit, reopen, carry on in the same conversations
  - [x] Killing the app mid-turn loses no state, and the next open recovers
  - [x] Rebuilding `target/` can't break running sessions
  - Rehearsal run by the integrator on 2026-11-06 against real Claude Code from a scratch install: two slices, one permission, one `ask_user`, a slice commit, quit and reopen (both conversations resumed with context, slices on first touch), `kill -9` mid-turn (no Claude process left; reopen raised the cut-off item). Hook and MCP paths pointed at the installed binary. A partial reply cut off by the kill is not kept in the conversation; Claude Code's own transcript has it. The missed redraw seen once in the first attempt did not reproduce.
  - [ ] Then repeated once on this repo, with your OK
- **Tests:** restore logic with `FakeDriver`, inbox reconciliation, kill and reopen

**From this point:** work runs inside Dovetail, using the installed release. Slices keep their own worktrees, and you still approve each milestone before the next starts. Dev builds in a slice must use separate data, config, state and runtime directories, never touching your live state. *As built (M10): through Dovetail's own `DOVETAIL_DATA_DIR`, `DOVETAIL_CONFIG_DIR`, `DOVETAIL_STATE_DIR` and `DOVETAIL_RUNTIME_DIR`, set per worktree by the recipe's `[env]` (`{worktree}`, `{worktree_hash}` placeholders), not `XDG_*`, which the slice's Claude Code session would inherit too (its files and messaging sockets would move). The worktree hook and MCP bridge are pinned to the live directories in the launch files.* If Dovetail breaks, plain `claude` in the repo still works, since everything is ordinary git.

### M11. Subagents and to-do lists
- **Scope:** subagents as tree nodes from `parent_tool_use_id` and `task_*`, progress, cost, unchanged subagent worktree removal, native list reading with reread on parse error, item-to-agent mapping, lists deleted on archive.
- **Slices:** (a) subagent routing and nodes; (b) task-file watcher and attribution.
- **Done when:**
  - [x] Background subagent shows progress, summary and cost (fake-driver replay of subagent_background / todo_tools; tokens stand in for cost per decision #21; not yet checked live)
  - [x] To-do items sit under the agent that created them (fake-driver replay of subagent_background / todo_tools; tokens stand in for cost per decision #21; not yet checked live)
  - [x] Totals come from task events (fake-driver replay of subagent_background / todo_tools; tokens stand in for cost per decision #21; not yet checked live)
- **Tests:** `subagent_background`, `todo_tools`, mid-write JSON parse error

### M12. Board and worktree view
- **Scope:** `<leader>b` board with `j k H L < > a s Enter`. `<leader>g` worktree view with sync state, changes, PR, setup, `Enter e c X`. nvim hand-off with TUI suspend.
- **Slices:** (a) board; (b) worktree data in core; (c) worktree buffer, after (b).
- **Board columns:** todo, in progress, done. A task that needs you gets a marker on its card, with no column of its own (B4).
- **Done when:**
  - [x] Both are reachable as buffers in the main region
  - [x] `X` asks first if work is unpushed
  - [x] `e` opens nvim and returns cleanly (hand-off tested with a fake core and editor; confirmed by the user in a real terminal, 2026-11-09)
- **Tests:** snapshots, git status parsing in temp repos

### M13. Memory caps and context fill
- **Spikes needed:** S4
- **Scope:** `systemd-run` launch, caps from config, memory per session and total, OOM detection into a Needs-you error, automatic restart from the session id with in-flight subagent work listed. Real context fill with amber and red.
- **Slices:** (a) caps and memory display; (b) OOM recovery, after (a); (c) context fill.
- **Done when:**
  - [x] A forced OOM kills one session only and recovers (kill: `a_forced_oom_kills_one_scope_only` with `DOVETAIL_CGROUP_TESTS=1` on the user's machine, 2026-11-09: a memory eater under a 64 MiB cap dies by SIGKILL, the `memory.events` watch sees `oom_kill` before the scope goes, and a second capped scope runs on. Recovery: fake-driver test, a session dying by SIGKILL after an `oom_kill` rise is resumed from its session id with its running subagent listed. Not yet checked with a real Claude session)
  - [x] Status bar shows total memory and coordinator `ctx` (confirmed by the user in the running app, 2026-11-10)
  - [x] Peak memory of this repo's build and test runs is recorded against the B8 caps (docs/spikes/cgroup.md: worst cold run 2735 MiB peak, 1.5 GiB anon, under the 3 GiB slice cap)
  - [x] Missing cgroup support gives a warning, not a failure (startup probe; failure path not yet tried on a machine without user systemd)
- **Tests:** `memory.events` parsing with fixtures, context calculation

### M14. Take-over
- **Merged:** PR #34 (task #32); migration `0011_agent_takeover`.
- **Scope:** `<leader>st` stops the headless process, suspends the TUI, runs native `claude --resume <id>` in the same terminal with the same settings and MCP config, then resumes headless on exit. Session lock. Note in the conversation that native turns happened.
- **Slices:** one slice; touches session lifecycle.
- **Done when:**
  - [ ] Round trip works from a real slice in its worktree (for the user to try; the steps are in PR for task #32; left unticked because the real-session round trip is not yet confirmed)
  - [x] Headless can't start while locked, including after a crash (fake core and fake `claude`: a prompt, a second take-over, archive and worktree removal are refused while native runs; a stored lock whose native process still runs keeps headless down after reopen, one whose process is gone is released. Not yet checked with a real session)
  - [x] Exiting native never removes a worktree (fake `claude` in a slice's worktree: the worktree and the slice are still there after it exits, and headless resumes in it. Not yet checked with a real session)
- **Tests:** lock state machine, crash recovery of the lock

### M15. Maps stage 1
- **Spikes needed:** S5
- **Scope:** `dovetail-maps`: model, nine kinds, `draw_map` validation with specific errors, `MapRenderer`, Mermaid conversion, swimlane grid layout. kitty canvas with node selection, flows, pan and zoom. Outline beside it and as the fallback. Map shelf. MAP mode with mouse only there. Restore the `draw_map` line in the coordinator prompt.
- **Slices:** (a) model and validation; (b) tool and storage, after (a); (c) merman renderer, after (a); (d) swimlane layout, after (a); (e) outline, after (b); (f) canvas and MAP mode, after (c) and (e).
- **Progress:** (a) merged, PR #32 (task #34). (b) merged, PR #35 (task #35); its maps migration is `0012_maps`, after `0011_agent_takeover`. (c) merged, PR #39 (task #5); (d) merged, PR #38 (task #6); (e) merged, PR #42 (task #7). (f) merged, PR #47. (b): `draw_map` for both roles, schema from `dovetail_maps::input_schema`; reusing an id stores a new version and a map keeps its type; a new map's id is its title's slug; lane `node` links are checked against the latest structure maps; the shelf is `Snapshot::maps` with `Event::MapUpserted`, content through `Command::LoadMap` → `Event::MapLoaded`; the mock seeds one structure and one swimlane map; the coordinator prompt's `draw_map` line is back. (e): the map shelf (`<leader>M`, `:maps`, newest first) and the outline buffer in `dovetail-tui`: `Enter` loads a map, `o` folds a group, `f` switches flow, `]]`/`[[` walk a flow's steps, `Esc` goes outline → shelf → conversation; a newer version of the open map is refetched in place. (f): the kitty canvas and MAP mode. The binary hands the event loop a painter (`dovetail_maps::MapRenderer` behind `dovetail_tui::canvas::MapPainter`, so merman stays out of the TUI); it paints each map version once per terminal size on its own thread, at twice the screen's pixel size (capped at 4096 px), and pan and zoom move kitty's source rectangle over that cached picture. Swimlanes are drawn by Dovetail as SVG on the (d) grid and rasterised with `resvg`. The selection is a ring drawn as a second small image over the picture, not a re-render with a highlight class: 0.06 ms against 56–184 ms (S5), and the layout does not move when classes change. The kitty commands are hand-written (`canvas::kitty`, base64 in 4096-byte chunks, `q=2` so no replies reach stdin). Graphics are detected from the environment (`TERM=xterm-kitty`, `KITTY_WINDOW_ID`, WezTerm, Ghostty; never inside tmux or screen; `DOVETAIL_GRAPHICS=kitty|none` overrides) plus the terminal's pixel size; without them the outline is the whole view. MAP mode (`Mode::Map`, status bar `MAP`): opening a map from the shelf enters it, `<leader>v` enters it on the open map, `Esc` leaves; the map takes the full width with the outline beside it (`v` hides it); `hjkl` and the arrows select the nearest node that way, `Tab`/`Shift-Tab` follow the first connection out or in, `Enter` drills into a group (zooms to its box) and `Backspace` comes back, `n`/`p` walk the flow, `f` switches it, `+`/`=`, `-`, `0` zoom, `Ctrl-hjkl` pan, `u` asks the coordinator to redraw after a yes; `c` (comments, v1.1) and `s` (B6) stay unbound. Mouse: click selects, the wheel zooms around the pointer, a drag pans.
- **Done when:**
  - [ ] A real agent draws both map types
  - [ ] Works in kitty; outline works elsewhere (yours: see `docs/plan/v1-status.md`, "Try the map canvas")
  - [x] Mouse capture is on only in MAP mode (PR #47; `dovetail-tui/tests/map_canvas.rs` `mouse_capture_is_on_in_map_mode_only` walks `App::wants_mouse` through MAP, Normal, Command and Insert mode into `MouseCapture::sync`, the call the event loop makes after every message, and checks the `?1000h`/`?1000l` it writes; mouse events outside MAP mode are ignored)
- **Tests:** `draw_map` validation, Mermaid output snapshots, grid layout, hit-testing

### M16. Hardening and daily-driver polish
- **Scope:** a real week's use, fixing what hurts. Long-session performance and memory. Startup warnings. Focus mode (A1). Activity feed events moved from M7: permission answers, coordinator start and exit, task changes, PR and CI (decided 2026-11-06). README and docs brought up to date.
- **Slices:** one per defect or theme, decided from real use.
- **Built (2026-11-11, branch `claude/m16-activity`, no box ticked by it):**
  - Activity events: `ActivityKind::Permission` and `::Task` added (`Pr`, `Ci`, `Lifecycle` existed). The core adds an item for each permission answer (`inbox.rs`), coordinator start, resume and exit (`session/mod.rs`; a crash keeps its existing "crashed" line), every task creation and patch except `Seen` (`describe_task_patch` in the API crate, shared with the mock), and a PR that opens, merges, closes or reopens (`worktree_info::pr_change`, from the `gh pr list` result M12 already gathers; the first lookup after start is a baseline, not news). The mock emits the same items. **CI is not emitted by the real core:** nothing Dovetail observes carries CI status (`gh pr list` is asked for number, state and url), and polling `gh pr checks` is a mechanism the design does not specify (question in this PR).
  - Startup warnings (`startup.rs`, pure functions with tests, one warning each, the app keeps running): messaging blockers, missing role prompts, **new:** missing recipe of a registered project, **new:** `claude --version` missing, unrunnable or older than 2.1.224 (run once after the first snapshot, off by default in tests via `CoreOptions::check_claude`). Already there: no cgroup support (`memory-caps`), non-`auto` init mode (`mode-<agent>`), unregistered project.
  - Long-session memory: the rendered-markdown cache of a conversation kept every entry ever seen (trimmed entries included) and views of removed agents stayed; both now go. The `@` popup keeps at most 32 directory listings and the core at most 4096 message ids (dedup). Everything else was already bounded: client conversation 2 000 entries (`CONVERSATION_KEEP`), snapshot tail 200, activity 500, messages 500, core tails 200.
  - Measurement: `scripts/rss-check.sh` (`crates/dovetail-tui/tests/rss.rs`, ignored in `cargo test`) streams 10 000 turns (≈ 600 000 deltas, ≈ 30 000 entries, a frame per turn) through the UI on synthetic events. **UI state: 5.4 MB at start, 67 MB at the end and at peak** (2 500 turns gave 65.8 MB, so it plateaus at the 2 000-entry cap). That is the TUI process's share only; the core's share (SQLite, sessions' stdout buffers) is not measured here, and the box below needs the real number after a day.
- **Done when:**
  - [ ] You use it for a full working week on a real project in place of tmux
  - [ ] Every item in the digest's v1 roadmap is shipped, or listed by you as cut
  - [ ] App RSS under 100 MB after a day's use (proposed target)

### R1. Research capture (core) — M17 of `docs/plan/research-sessions.md`
- **Scope:** `docs/research.md` §3–§5 and ADR 0015, following spike S7 (`docs/spikes/research-capture.md`). Rounds, findings and sources in `state.db` (migration `0013_research`); automatic capture from the stream with nothing sent to any session; files under the research root, rebuilt at start when missing; the recipe's `research_dir` opt-in. No MCP tools: a round is opened with `Command::StartRound` (`:round <task> <brief>`) and closed with `Command::CloseRound` (`:round close`).
- **Built (branch `dovetail/m17-research-capture`):**
  - API (`dovetail-api/src/research.rs`): `ResearchSummary` per task in `Snapshot::research`, kept current by `Event::ResearchUpserted`; content through `Command::LoadResearch { task, what: ResearchWhat }` → `Event::ResearchLoaded { task, what, content: ResearchContent }` (a `Listing` of findings and sources with absolute paths, or one markdown `Document`). Ids `FindingId`, `SourceId` are the row ids (`F<id>`, `S<id>` for R2's citations).
  - Core (`dovetail-core/src/research/{mod,capture,files}.rs`, `db/research.rs`): at most one round is open; while it is, every subagent the coordinator launches is watched. A session working on a task gets quick findings (no round) for subagents that used `WebFetch`/`WebSearch`. Sources come from the subagent's `WebFetch`, `WebSearch` (each `Links:` URL, plus the search itself as an untrusted `search` source), `Read`, `Grep`/`Glob` (listed, not cached) calls and their results; one `sources` row per locator and content, a `finding_sources` row per tool call. The body follows S7's order (notification handback, `Agent` result handback, the `SubagentHandback` call, `Agent` result content, assistant text, a non-handback summary, the output file's `assistant` lines as a last resort). A finding is filed on `task_updated`/`task_notification` or the foreground `Agent` result, never on a `result`, and a better body later rewrites it.
  - Files: default root `<data>/projects/<id>/research/`, or the recipe's `research_dir` (relative to the repo root, no `..`; or absolute). `config/projects/dovetail.toml` sets `docs/research`; `scripts/bootstrap.sh` reports whether the installed recipe has it.
- **Done when:** the M17 validation list in `docs/plan/research-sessions.md`.
- **Tests:** `dovetail-core/tests/research.rs` (both S7 recordings through `FakeDriver`, a page fetched twice, restart with files rebuilt, the opt-in folder, no round), `research::capture` unit tests (quick findings, WebSearch links, the output file), `dovetail-tui/tests/research.rs` (`:round`).

### R2. Research suite tools — M18 of `docs/plan/research-sessions.md`
- **Scope:** `docs/research.md` §6–§7 and ADR 0015. Four coordinator-only tools served by `dovetail mcp` with the `research_` names as written (the suite rename is M20): `research_start_round`, `research_write_report`, `research_propose`, `research_get`.
- **Built (branch `dovetail/m18-research-tools`):**
  - API: the four tools in `dovetail-api/src/tools.rs` (`ResearchStartRound`, `ResearchWriteReport`, `ResearchPropose`, `ResearchGet`; a decision without `evidence`, a malformed id or a bad `what` fail `Tool::validate`); `research::{Evidence, ProposalKind}` (`Evidence` is written `F3`/`S12` on the wire); `InboxKind::Proposal { task, kind, text, evidence, why, alternatives }`; `Decision::evidence`.
  - Core: `tools/research.rs` (handlers and the answer to a proposal), `research/report.rs` (report validation: unresolved citations and a decision report without its sections refuse; uncited claims and unlabelled figures flag; the `Evidence: N sources, M independent` line is rewritten from the cited sources, searches excluded, independence by registrable domain or file path), `Core::open_round_for`/`close_round_for` (the one path under `Command::StartRound`/`CloseRound` and the tools), `Waiter::Proposal`. Migration `0014_decision_evidence`; `Db::{insert_report, research_kind, set_research_kind, log_decision_with_evidence}`.
  - `research_propose` returns `pending` at once and writes nothing. `y` (`PermissionAnswer::Allow`): an idea becomes a follow-up task in Unsorted (`follow_up_of` the research task, not marked coordinator-created), a decision goes through the `log_decision` storage path with its evidence rows. `A` (`AllowAlways`) on an idea opens a new round with the idea as its brief. `d` and `x` drop it. The coordinator is told by a user message either way (as `start_slice`); a proposal pending at a restart expires with a message, like `start_slice`.
  - TUI (minimum): the Proposal item in Needs you (row, detail, switcher; `y`/`A`/`d`) and `evidence: [F3] [S12]` under a decision in the task detail.
  - `config/roles/coordinator.md` gained two research lines (18 lines); the installed copy needs `scripts/bootstrap.sh --force`.
- **Done when:** the M18 validation list in `docs/plan/research-sessions.md`.
- **Tests:** `dovetail-core/tests/research_tools.rs` (round and report through the socket and `FakeDriver`, the specified errors, accept and drop, the idea routes, restart), `research::report` unit tests, `dovetail-api` serde test for `Proposal` and `Evidence`.

### R3. Rough reading (TUI) — M19 of `docs/plan/research-sessions.md`
- **Scope:** `docs/research.md` §9–§10. Read a task's research inside Dovetail: the report, round briefs, findings and sources, rendered in the existing reading column. No new layout, no new buffer kind and no `<leader>` change: it all lives in the task detail (`BufferKind::TaskDetail`).
- **Built (branch `dovetail/m19-research-reading`):**
  - `buffers/task_research.rs`: the "Research" section of the task detail (counts, report row or "no report yet", each round with its brief and findings, quick findings, sources) from `Snapshot::research` plus the listing the UI asks for with `LoadResearch { Listing }` when a task with research is opened (and again on `ResearchUpserted` for the open task).
  - `buffers/task_detail.rs`: with research rows present `j`/`k` move a selection between them (the view follows); `PageUp`/`PageDown`/`gg`/`G` scroll or jump; `Enter` (`Action::ResearchOpen`, `DETAIL` scope only) sends `LoadResearch { what }` and, when `ResearchLoaded` answers, the same buffer shows the document through `render/markdown.rs` in `render/reading_column.rs` (120 columns, centred, solid background follows `<leader>B`). `Esc` goes back to the detail first, then out as before. In the document, `e` (already bound in the `DETAIL` scope) hands the file to the editor through `Effect::OpenEditor` / `Command::OpenEditor`, the existing core hand-off; a listed-but-not-cached source says it has no file.
  - `render/citations.rs`: `[3]`, `[F3]`, `[S2]` and lists of them (`[F1, S2]`) coloured with the new `Theme::cite`, also when the renderer splits a marker over spans.
  - `:research` (`Action::ListResearch`): a picker over the tasks that have research; choosing one opens its detail.
  - Mock: task #3 is seeded with two rounds, three findings (one quick), three sources and a report, and `LoadResearch` is answered (`dovetail-mock/src/research.rs`), so `dovetail --mock` shows everything.
- **Done when:** the M19 validation list in `docs/plan/research-sessions.md`.
- **Tests:** `dovetail-tui/tests/research.rs` (rows listed, every artifact kind opens, coloured markers, `Esc`, `e` hand-off, `:research`, snapshots of the detail and of a report at 80, 120 and 200 columns), `render::citations` unit tests, `dovetail-mock/tests/mock.rs` (every listed document loads).

### Research-only projects (decided 2026-11-11)
- **Scope:** ADR 0015's open point, decided by the user on 2026-11-11 (the ADR text is not updated yet; this wins): the registry gets a project kind, `code` or `research`. A research project has no repo; its sessions run in its state folder `<data>/projects/<id>/`. Code paths check the kind, never whether a folder exists. Also B24, the project switch, which was a stub.
- **Built (branch `dovetail/research-projects`):**
  - Registry migration `0003_project_kind`; `projects::Project { kind, repo_path: Option<_> }` with `folder(&Paths)` (repo, or `Paths::project_dir`) and `summary(&Paths)`; `Db::{register_research, get_project}`; `projects::open` (by id). `ProjectSummary::kind` (`ProjectKind`, serde default `code`); `repo_path` there is the folder sessions run in.
  - Core kind checks (`ProjectState::is_research`): `check_slice` refuses with `SliceError::ResearchProject` (so `start_slice`, `:spawn` and queued starts); worktree gathering, the stale-state sweep, subagent worktree cleanup and `@`-completion's `git ls-files` are skipped; `:upgrade` refuses; no "no recipe" warning; the research root is always the default `<data>/projects/<id>/research/` (a recipe's `research_dir` is ignored). The recipe is still read for a `profile` (ADR 0009).
  - `Command::NewResearchProject { name }` registers one (refused when a research project of that name exists) and makes its folder; a fresh snapshot lists it. `Command::OpenProject { id }` (B24): the core records the id (`CoreHandle::switch_requested`), sends `Event::OpeningProject` and quits; the UI quits on it; the binary then `exec`s itself with `--project <id>` (in the project's folder for a code project). `dovetail --project <id>` opens any registered project, research ones included.
  - TUI: `<leader>pp` / `:project` opens the project picker (`switcher::project_items`, `PickerKey::Project`): research projects marked `◇` and `research` where a code project shows its folder, the open one `open ·`, a preview per project; choosing one asks first. `:project new <name> --research` (`Action::NewProject`). `<leader>g` in a research project says it has no worktrees instead of opening the view.
  - Mock: "Market sizing" is seeded as a research project; `:project new … --research` adds one to the list; opening another project is not simulated.
- **Done when:** a research project can be created, opened from the switcher, chats in its state folder with the coordinator role prompt, runs rounds, findings and reports, closes the day, and comes back after a restart; slices, worktrees and git never run for it.
- **Tests:** `dovetail-core/tests/research_projects.rs` (coordinator cwd and role prompt, restart resumes it there, slices refused by command and tool, no worktree gathering, no git listing, `:upgrade` refused, `NewResearchProject` and `OpenProject`), `db::tests::an_existing_registry_migrates_to_project_kinds`, `dovetail-tui/tests/projects.rs` (switcher marker, pick asks then sends `OpenProject`, `:project new`, no worktree view), `command` unit test, serde round trip.

### Daily-driver fixes (after M10)
Fixes from the first days of building Dovetail inside Dovetail, one slice and PR each, all merged on 2026-11-09. They come ahead of M11 and count towards M16's "fixing what hurts"; no milestone box is ticked by them.

| Task | Fix | PR |
| --- | --- | --- |
| #1 | `Ctrl-V` pastes clipboard images into the chat bar as attachments | #5 |
| #2 | `ask_user` never blocks; a chat reply reaches the agent at once and clears its open questions (ADR 0007) | #6 |
| #4 | The chat bar wraps at word boundaries, not mid-word | #4 |
| #5 | Inbox rows are one line; `Enter` opens the item in full in an `InboxDetail` buffer (finished turns still open the conversation) | #8 |
| #6 | A header rule opens each conversation message | #7 |
| #7 | Starting a slice on a task moves the task to Now | #9 |
| #9 | Coordinator-only `archive_slice { name }` and `archive_task { id }` tools, through the same archive path as the UI: clean, pushed work goes at once (`archived`, the queue place freed); unsaved work raises the usual Permission item (`waiting_for_user`, the user's answer reaches the coordinator as "slice <name> archived" or "… kept"). The role prompt archives after merge and pull | #11 |
| #11 | M11a: subagents as tree nodes, from `parent_tool_use_id` and `task_*` events, with progress, summary and cost; unchanged subagent worktrees removed | #12 |
| #12 | M11b: native to-do lists under the agent that created them, watched with inotify and reread on a parse error; lists deleted on archive | #13 |
| #13 | Fixes from the `review-wave-2` code review: double `Ctrl-C` asks while a turn runs, SIGHUP is treated like SIGTERM, clipboard and image saving move to the core | #16 |
| #14 | The inbox sits over the tree in the side column (ADR 0008) | #14 |
| #15 | Text colours softened ~13 % for dark translucent terminals | #15 |
| #16 | The header shows the model and the 5-hour and weekly usage limits (decision #25) | #17 |
| #17 | Fixes from the wave-3 code review: subagent worktree cleanup keeps the worktree when git's output does not parse; to-dos of a pruned subagent fall back to the list owner; a session's subagent tokens summed from task events and shown in the switcher; code smells | #18 |
| #20, #25 | Coordinator task tools: `list_tasks` and `get_task` read the tracker; `update_task` changes status, bucket, priority, title and notes, but never moves a task into Now (only starting a slice does, fix #7) | #20, #25 |
| #3 | A bracketed paste or dropped file that is a path to an existing PNG, JPEG, GIF or WebP (`~`, `file://` and shell quoting understood) attaches like `Ctrl-V`, through `Command::PasteText` and the core; anything else stays text | #45 |
| #8 | Saved paste attachments are cleaned up. Policy: a file a live agent's messages name is never deleted; an agent's files go when it is archived; at startup files older than 7 days that no live agent in any project names are removed (the sweep is skipped if any project database cannot be read). Only files directly in `attachments/` are touched | #45 |
| — | Chat UX from daily use (2026-11-10): message headers in the speaker's dimmed colour with more space between speakers; Shift-Enter newline, `Ctrl-k`/`Ctrl-j` between chat bar and conversation, `PageUp`/`PageDown` while typing (ADR 0010); `/` and `@` completion and `:skills` (docs/spikes/slash-and-at.md); focus mode `<leader>z` (A1, from M16); side chats `<leader>sq`/`:ask` and `<leader>y` (ADR 0011) | merged directly, no numbered PR: 1a36dc1 (`dovetail/chat-ux`: chat bar keys, completion, focus mode, message headers), 5f1abb7 (`dovetail/chat-ux`: side chats), 007af85 (`dovetail/chat-pane`: chat bar as its own pane, ADR 0010), 6b009ab (`dovetail/focus-spacing`: focus mode keeps the background, ADR 0013) |
| — | End of day and a fresh morning coordinator (ADR 0012, pulled from v1.1's decisions 17 and 22): `journal/YYYY-MM-DD.md` with the coordinator's Handoff and Dovetail's Facts; `:eod`; "Close the day?" on quit (`y` closes then quits, `n` quits now, `Esc` later); on the first open of a new day a new coordinator session seeded with the newest journal file and the open tasks, with a Needs-you item whose `y` keeps yesterday's session instead | #36 (merge bf64d2b) |

| — | The side panel (decided 2026-11-11, over ADR 0011 and ADR 0017): `<leader>?` shows side chats in the side column in place of the tree, beside the coordinator's conversation; `<leader>sq`/`:ask` open there; `Tab`/`Shift-Tab` switch, `dd` closes, the panel has its own chat bar; focus mode hides it. Side chats are read-only for real: `--permission-mode dontAsk`, Bash allowed only for a read-only command list (`SIDE_CHAT_BASH` in `session/launch.rs`) with writing options denied, mode never cycles. `<leader>y` removed. `--mock` seeds `side-1` | `dovetail/side-panel` |

**Open follow-ups:** task 3 builds ADR 0014 (upgrade from inside the app: `:upgrade` and `dovetail upgrade`, on main).

---

## 7. Where the docs disagree or say nothing

`AGENTS.md` says to stop and ask on these, so nothing here is assumed settled. I'll apply each default unless you change it. **Your call** marks UX or scope choices, where your answer matters most.

### A. Docs disagree

| # | Issue | Proposed default |
| --- | --- | --- |
| A1 **Your call** | **Focus mode:** v1 in README and the UI brief; v1.1 in the roadmaps and `screens/README.md` | **Decided: v1, as a small M16 item** (`<leader>z`): coordinator conversation and input only, solid background on, Needs you count kept. The solid-background toggle itself ships earlier |
| A2 **Your call** | **`create_task`:** in v1 roadmaps under quick capture; v1.1 in Decision 25 and both tool tables | **Decided, refines Decision 25 ([ADR 0002](adr/0002-create-task-in-v1.md)):** v1, at M9. Coordinator only; tasks land in Unsorted, marked coordinator-created until seen |
| A3 | **Board and worktree view:** v1 in README and screens; missing from both v1 roadmap lists | v1 (M12) |
| A4 **Your call** | **Task detail view:** v1.1 on the navigation screen, but quick capture's `Tab` (v1) and tree `Enter` need it | **Decided:** v1 with fields, notes and agents. **Refined 2026-11-03 ([ADR 0003](adr/0003-decision-log-in-v1.md)): the decision log is in v1 too** (view and mock at M4, storage at M5, `log_decision` at M9). `e` opens notes in `$EDITOR` through a temporary file, with the TUI suspended, and saves them back on exit |
| A5 | **Notes storage:** a `state.db` field in `design.md`, not in the digest's list, and `tasks/<id>.md` in Decision 19c (v1.1) | `tasks.notes` column in v1, migrated to files in v1.1 |
| A6 | **Picker scope:** "sessions, inbox and messages" (design, README); "tasks, agents, inbox items" (brief) | Tasks, agents, inbox, plus maps from M15; messages stay in the timeline |
| A7 | **Which-key:** Decision 10a locks it; roadmap says v1.1; the brief says only if it falls out naturally | v1.1. M2's pending-sequence state leaves room for it |
| A8 | **Subagent source:** hooks and transcripts in the design's table and roadmap; `task_*` events in Decision 11 and the digest | Stream only (Spike 1 is newer); no `SubagentStart`/`SubagentStop` hooks |
| A9 | **`WorktreeRemove`:** "handles removal" in places; Spike 4 says it doesn't fire in `-p` | Dovetail cleans up. Register a no-op hook so a native exit in take-over can't remove a worktree |
| A10 | **Conversation storage:** "referenced, not copied" (5b); revised by Decision 20, whose search is v1.1 | Store conversation text from v1, add FTS in v1.1 |
| A11 | **`i` target:** "from an open slice, that slice" (design); "coordinator, unless an agent is selected in the tree" (brief) | Brief's rule, plus: if the main region is showing an agent and has focus, target that agent |
| A12 | **Task list id:** `<task id>` in the digest, but lists are global in `~/.claude/tasks/` and the coordinator has a project list | `dovetail-<project>-task-<id>`, and `dovetail-<project>-coordinator` |
| A13 | **Spike 3's flag:** `--permission-prompt-tool stdio` is in `design.md` but not in the digest's launch list, and is now absent from `--help` | **Resolved by S1:** required. Without it a manual-mode request is auto-denied (`system/permission_denied`) and never reaches Dovetail, despite `--permission-prompts host`. It is in the launch flags above. `--verbose` and `--include-partial-messages` are in neither doc but required |
| A14 | **Mock vs docs:** always-allow shown as `a`; canvas shows `hjkl` as pan; `c comment · e export` on the v1 canvas | Docs win: `A`; `hjkl` selects and `Ctrl-hjkl` pans; comments v1.1, export v2 |
| A15 | **Stale text:** Decision 18 says "bottom-right" but the inbox is bottom-left; Visual Maps "Views" still mentions a text navigator; Decision 13 and 15–17 numbering repeats | Treat as stale: helper uses the inbox region; the version table wins; cite numbers as the digest does |
| A16 | **Run rule:** "never run real sessions against real projects" vs self-hosting on this repo | Applies to development and tests. Self-hosting uses the installed release, and live tests stay in scratch repos |
| A17 | **Memory reading:** "from `/proc`" in docs, but a process-level read misses a session's tools | Read cgroup `memory.current`, fall back to `/proc` PSS when uncapped |
| A18 | **Machine size:** docs say 16 GB; this machine reports 30 GB | Keep 16 GB as the budget |
| A19 | **Theme naming:** screens README says Twilight colours and Dusk as default; the navigation mock offers Iceberg, Catppuccin, Rosé Pine | Dusk as default theme struct values |

### B. Docs say nothing

| # | Gap | Proposed default |
| --- | --- | --- |
| B1 **Your call** | **Who starts slices?** No agent tool exists, yet the coordinator's prompt says to brief slices. This shapes the whole workflow | **Decided: you start them** (`<leader>sn` on a task; prompts for name and brief). The coordinator recommends slices in chat, then messages the slice. Decision 25 is unchanged, so no ADR. **Superseded 2026-11-05 by [ADR 0004](adr/0004-start-slice-tool.md):** the coordinator also starts slices with `start_slice`, following its permission mode, capped at 3 with a queue. Original upgrade note: a `start_slice` tool that raises a Permission request for your `y`/`d`, enforced by Dovetail's handler in every permission mode. It returns at once, needs a ready signal and a start queue, targets existing tasks only, and needs an ADR naming Decision 25 |
| B2 **Your call** | **`hjkl` vs lists** (the brief asks me to decide): how to move between tree and inbox when `j/k` move inside lists | **Decided:** `h/l` move between left and right; `h` returns to the left region you came from. `j/k` move inside and never leave a list. `Ctrl-j/k` move between regions in the column. `<leader>n` jumps to the inbox. `Ctrl-hjkl` pan only in MAP mode. `o` folds tree nodes, per the brief |
| B3 **Your call** | **Every finished turn in Needs you** may flood the inbox, especially from the coordinator you're watching | **Decided, refines Decision 7b ([ADR 0001](adr/0001-collapse-finished-turns.md)):** one item per agent (newest replaces), auto-cleared when you open that agent. None while background subagents remain. Other inbox kinds are never collapsed |
| B4 **Your call** | **`needs_you` as task status:** set by hand or derived? | **Decided: derived** from agents and inbox, never stored. `s` cycles todo → in progress → done. The board has no needs-you column: a needs-you task shows a marker on its card, in its status column (decided 2026-11-03). Starting a slice moves a todo task to in progress. Done wins: a done task stays done if an agent raises an item, which still shows in the inbox. Coordinator items mark no task |
| B5 **Your call** | **Slice setup vs "hook must be fast":** the hook runs setup, but setup can take minutes and the mock shows a live timer | **Decided:** hook runs setup once with a 15-minute hook timeout, recording state under `~/.local/state/dovetail/`; repeat starts return at once. Interrupted setup re-runs, so recipe setup commands must be safe to re-run. If M8 finds the timeout isn't honoured, the core runs setup after the hook instead, with an ADR naming 12b |
| B6 **Your call** | **Map shelf and map key:** the mock puts a shelf in the left column; no doc gives it a region; map mode `s` (Decision 24) has no meaning anywhere | **Decided:** shelf is a collapsed `▸ Maps (n)` row at the bottom of the task tree, hidden until a map exists. `<leader>M` unfolds it and moves the cursor there; `o` folds it; the picker also opens maps. A dedicated maps buffer can follow if the list outgrows the tree. `s` stays unbound in v1 |
| B7 **Your call** | **Merging:** no flow exists for slice branches reaching `main` (`:merge` appears once, undefined) | **Decided:** no merge command or tool. Slices push a branch and open a PR. On your yes, the coordinator runs `gh pr merge`, then `git pull --ff-only` so new slices branch from current `main` (B13). Launch settings add an ask rule for `Bash(gh pr merge:*)`, so each merge raises a Permission request. If M8 shows the rule doesn't hold in auto mode, fall back to the role prompt alone. You still mark the task done and archive it |
| B8 **Your call** | **Memory cap values** | **Decided:** coordinator 4 GB, slice 3 GB, in `config.toml`. Caps include tools, so `cargo build` counts; M13 records this repo's real peaks before any tuning. Later idea, not v1: size caps from the machine's memory and the target slice count |
| B9 | **Project id format** | Slug of the repo folder, numbered on collision. Matches the mock's path and bootstrap's `dovetail.toml` |
| B10 | **Global config** | `~/.config/dovetail/config.toml`: reading width, caps, `claude` path, default model |
| B11 | **Hook entry point:** `AGENTS.md` lists `mcp` and `add` only | Internal subcommand `dovetail hook worktree-create` |
| B12 | **Subagent worktrees and setup** | Symlinks yes, setup no, to keep them fast. Branch `dovetail/agent-<id>` |
| B13 | **Slice branch base** | Current `HEAD` of the main checkout; branch `dovetail/<slice>` |
| B14 | **Done and archived tasks** | Collapsed `Done (n)` at the tree bottom, per the mock; archived hidden |
| B15 | **Agent status for setup:** the mock shows `◌ setting up`; the data model has no such status | Add `SettingUp` |
| B16 | **`ask_user` schema** | `{ question: string }`; returns at once and the answer arrives as a user message (ADR 0007 removed `blocking`) |
| B17 | **`report_status` schema** | `{ status: string }`, one line, truncated to 120 characters |
| B18 | **Bridge identity** | Per-session MCP config passes `DOVETAIL_AGENT_ID` and the socket path by env |
| B19 | **App closed with `dovetail add`** | Uses the socket if the app is running; otherwise writes directly to SQLite with its own migration check |
| B20 | **`<leader>sm` on a subagent** | Applies to the parent session, with a status note |
| B21 | **`A` with no suggestions** | Disabled, with a status note |
| B22 | **`r` on a finished turn** | Sends a normal message to that agent |
| B23 | **Quit with live sessions** | Sessions end with the app (no daemon in v1). `:q` asks if a turn is running; everything resumes on open |
| B24 | **Project switch** | Stops the current project's sessions after asking |
| B25 | **Starting outside a registered repo** | ~~Offers to register; outside any git repo, opens the last project~~ Superseded by [ADR 0005](adr/0005-explicit-projects-any-folder.md): registered only on request (`:project add` or the coordinator's `add_project`), and any folder can be a project |
| B26 | **Take-over terminal** | Same terminal, TUI suspended, as the mock shows |
| B27 | **Key-sequence timeout** | 1000 ms |
| B28 | **`--strict-mcp-config`** | Not used, so your own MCP servers still load |
| B29 | **Context window source** | **Resolved by S1:** `result.modelUsage.<model>.contextWindow` is present (and `maxOutputTokens`); no model table |
| B30 | **Socket path** | `$XDG_RUNTIME_DIR/dovetail/<project-id>.sock`, mode 0600 |

---

## 8. Risks

| # | Risk | Plan |
| --- | --- | --- |
| R1 | **Pro plan limits** may stop several concurrent sessions, which could undercut self-hosting | Measure in M10's rehearsal. Prefer fewer slices; decide on plan or API billing with real data |
| R2 | **The undocumented protocol** can change between Claude Code releases | Driver isolated, unknown events tolerated, fixtures re-captured on upgrade, supported version noted |
| R3 | **A young driver crate** may lag behind | The trait isolates it; the hand-written fallback is small |
| R4 | **Self-hosting can break its own tool** | Stable install, upgrades only when idle, separate dev state, forward-only migrations, plain `claude` as the way back |
| R5 | **A replaced binary:** a rebuild under a running app breaks hook and MCP paths | Fixed install path; upgrades via restart (M10 test). `:upgrade` and `dovetail upgrade` (ADR 0014) build into a staging folder and rename over the install only while no app and no helper runs |
| R6 | **Shell writes aren't blocked** from slices into the main checkout (Spike 4) | Role prompt only in v1, as designed. Accepted risk, and it matters more while self-hosting |
| R7 | **Untested auto-mode behaviour:** classifier denials and escalations | Handled generically via the inbox; fixtures added when first seen |
| R8 | **Upgrades interrupt running turns**, since there's no daemon | Upgrade when idle; M10's restart recovery covers the rest |
| R9 | **merman is pre-1.0** and may add real weight | S5 measures it; `dovetail-maps` isolates it |

---

## 9. Verification of this plan

When saved, it's correct if:
- [ ] `docs/implementation-plan.md` has all seven sections `AGENTS.md` requires
- [ ] Every open spike in the digest has a procedure, pass and fail criteria, fallback and owner
- [ ] The TUI is usable on the mock (M4) before any Claude Code integration (M6)
- [ ] The self-hosting point (M10) covers everything in the `AGENTS.md` minimum
- [ ] Every milestone leaves something you can run
- [ ] Every later milestone names its slices

**Next steps for you**
1. Review the spike results: S0 passed on 2026-11-01 (`docs/spikes/terms-of-use.md`); S1, S3, S4 ran on 2026-11-02 (`docs/spikes/driver.md`, `mcp.md`, `cgroup.md`); S2 is in `messaging.md`. Then give the OK for M1.
