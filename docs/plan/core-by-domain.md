# Plan: Restructure `dovetail-core` by domain (task #59)

**Decision:** D44 (from task #48, review of `dovetail-core`).
**Branch rule (D37):** all work lives on the long-lived `restructure` branch. Never work on `main` and never push to `main` until the user decides to merge `restructure`.
**Gate:** `CARGO_BUILD_JOBS=2 scripts/check.sh` must be green at every step.

## Goal

Reorganize `crates/dovetail-core/` from ~50 flat files around a 2,000-line `service.rs` into four infrastructure modules and eight domain modules mirroring `dovetail-api`:

- **Infrastructure:**
  - `engine/` — the single-threaded core loop (`Core`, `Input`, `CoreOptions`, `CoreHandle`, `spawn`, thin command/tool/driver routers, startup checks, shutdown).
  - `storage/` — the dedicated SQLite DB thread (`Db`, `DbError`, typed channel), `migrations/` (`state/` and `registry/`), and cross-domain summary queries (`counts.rs`, `registry.rs`). Domain-specific SQL lives next to each domain.
  - `sessions/` — session process lifecycle, kept separate from `agents/`: `Drivers`, launch (`claude`, `agy`), stream state, restore, takeover, cgroup memory caps, OOM watcher, context window/fill tracking, and session DB rows.
  - `mcp/` — Unix socket server (`socket.rs`), `CoreHandler`, and thin MCP tool dispatch (`mod.rs`). Domain tool handlers live inside their owning domains.
- **Domains (mirroring `dovetail-api` + `settings/`):**
  - `settings/` — `config.rs`, `recipe.rs`, `profile.rs`, `paths.rs`.
  - `app/` — `clipboard.rs`, `editor.rs`, `external.rs`, `foreground.rs`, `upgrade/` (`mod.rs`, `flow.rs`, `tests.rs`).
  - `code/` — `worktree.rs`, `worktree_info.rs`.
  - `visual/` — `canvas.rs`, `design.rs`, `preview.rs`, map tool handlers (`tools/map.rs`), plus visual SQL (`db/canvas.rs`, `db/design.rs`, `db/maps.rs`).
  - `research/` — `capture.rs`, `files.rs`, `report.rs`, `ask.rs`, `browse.rs`, `docs.rs`, research tool handlers (`tools/research.rs`), plus research SQL (`db/research.rs`).
  - `activity/` — `inbox.rs`, `usage.rs`, activity/message feed & warning helpers.
  - `tracking/` — `projects.rs`, `project_admin.rs`, `registration.rs`, `home.rs`, `efforts.rs`, task/decision/effort command & tool handlers, plus tracking SQL (`db/efforts.rs`, `db/tasks.rs`).
  - `agents/` — `slice.rs`, `tangents.rs`, `journal/` (`mod.rs`, `switch.rs`), `todos.rs`, `attachments.rs`, `completion.rs`, `side_chat.rs`, `report.rs`, `agy_models.rs`, and agent/conversation/model/mcp command & tool handlers.

Additional goals folded in:
- **`Core` state structs:** `Core` holds one state struct per domain (plus infrastructure handles); `Core::command`, `Core::tool`, and `Core::on_driver` become thin routers delegating to domain methods.
- **Full `dovetail_api` domain paths (D43):** switch `dovetail-core`'s `dovetail_api` imports to full domain paths (`dovetail_api::<domain>::<feature>::...`, e.g., `dovetail_api::tracking::task::Task`, `dovetail_api::code::worktree::WorktreeInfo`, `dovetail_api::app::AppCommand`).
- **D42 library/error cleanup:** all 35 error types in `dovetail-core` already derive `thiserror::Error`. Remaining cleanup is dissolving `error.rs` (moving `SessionError`, `SliceError`, `SetupFailure`, `RegisterError` into their owning domains) and tidying manual `From` impls in `clipboard.rs` and `tangents.rs` where `#[from]` is cleaner.
- **Tests split by domain:** unify the duplicated test harness into `tests/common/mod.rs`, then split the ~7,000-line `tests/sessions.rs` into one integration test file per domain.
- **Architecture doc:** write `crates/dovetail-core/ARCHITECTURE.md` and include it in `crates/dovetail-core/src/lib.rs` via `#![doc = include_str!("../ARCHITECTURE.md")]`.

## `Core` state grouping

Today `Core` in `service.rs` holds ~45 flat fields. By Step 7 they group into infrastructure fields plus one state struct per domain:

| Owner | Fields currently on `Core` |
| --- | --- |
| `engine/` (on `Core` directly) | `db`, `paths`, `config`, `options`, `out`, `inputs`, `seq`, `ui_alive`, `quitting`, `killed`, `next_sample`, `switch_to` |
| `mcp::McpState` | `socket`, `extra_sockets` |
| `sessions::SessionState` | `session_prefix`, `live`, `generation`, `takeover`, `held_for_takeover`, `context_windows`, `claude`, `scopes`, `oom` |
| `tracking::TrackingState` | `project`, `efforts`, `effort_tags`, `sent_back`, `reattach` |
| `agents::AgentState` | `agents`, `tails`, `next_entry`, `deltas`, `flush_at`, `queue`, `archive_calls`, `archive_told`, `closed_slices`, `coordinator_notes`, `notes_wait_for_user`, `todos`, `commands`, `closing`, `pending_fresh`, `tangents`, `model_catalog` |
| `activity::ActivityState` | `inbox`, `next_inbox`, `activity`, `next_activity`, `messages`, `next_message`, `seen_messages`, `warnings`, `totals`, `usage` |
| `visual::VisualState` | `previews` |
| `research::ResearchState` | `research` |
| `code::CodeState` | `worktrees` |
| `app::AppState` | `editor_open`, `editor_pending`, `upgrade` |

## Incremental steps

Each step lands as one or more clean commits on `restructure` with `CARGO_BUILD_JOBS=2 scripts/check.sh` green.

### Step 1 — Unify test harness in `tests/common/`
- [x] Extend `crates/dovetail-core/tests/common/mod.rs` minimally so all integration tests can share one `Harness` and `fixture()` helper without losing coverage:
  - Track `ResearchEvent::Loaded` (`loaded: Vec<(TaskId, ResearchWhat, ResearchContent)>`) and general `events: Vec<Event>` if needed, plus `wait_until` and `OPENER` thread-local (or builder/helper) for `research_tools.rs`.
  - Add a `Harness::start_project(driver, paths, project)` (or `start_with_paths`) constructor that preserves custom `Paths` and `Project` (including research-only projects with no repos) and `socket()` / `wait_for_note()` helpers.
- [x] Migrate `tests/research.rs`, `tests/research_projects.rs`, `tests/research_tools.rs`, and `tests/tangents.rs` off their local `App` / `fixture()` / `socket_call()` duplicates onto `mod common;`.

### Step 2 — Move leaf domains (`settings/`, `app/`, `code/`)
- [x] `settings/`: move `config.rs`, `recipe.rs`, `profile.rs`, `paths.rs` into `src/settings/` with `src/settings/mod.rs`.
- [x] `app/`: move `clipboard.rs`, `editor.rs`, `external.rs`, `upgrade/` (`mod.rs`, `flow.rs`, `tests.rs`) into `src/app/` with `src/app/mod.rs`. Tidy `clipboard.rs`'s manual `From<RunError> for ClipError` if `thiserror` `#[from]` makes it cleaner (kept as-is: `RunError` maps 3 private variants onto 3 distinct public `ClipError` variants without an extra wrapper).
- [x] `code/`: move `worktree.rs`, `worktree_info.rs` into `src/code/` with `src/code/mod.rs`.
- [x] Keep temporary `pub use` re-exports in `src/lib.rs` (`config`, `paths`, `profile`, `recipe`, `editor`, `external`, `upgrade`, `worktree`) so `crates/dovetail` and `tests/` do not churn until Step 7.
- [x] Switch `dovetail_api` imports inside all moved files to full domain paths (`dovetail_api::app::...`, `dovetail_api::code::worktree::...`, `dovetail_api::agents::conversation::...`, etc.).

### Step 3 — Move `storage/` (+ atomic migration path update)
- [x] Move `src/db.rs` and `src/db/` into `src/storage/` (`src/storage/mod.rs`, `src/storage/migrations/`, `src/storage/counts.rs`, `src/storage/registry.rs`, plus the domain SQL files temporarily under `src/storage/` until their owning domains move in Steps 4–6).
- [x] Atomically update all migration path references in the same commit:
  - `include_str!` macro in `src/storage/mod.rs` (`migrations/...`),
  - `migrations_are_contiguous_and_listed_in_order` in `src/storage/mod.rs` (`src/storage/migrations`),
  - `scripts/check.sh` (`mig=crates/dovetail-core/src/storage/migrations`, checking both `storage/migrations` and `db/migrations` on `origin/main`).
- [x] Keep `pub use storage as db;` temporarily in `src/lib.rs` until Step 7.

### Step 4 — Move `sessions/` and `mcp/`
- [x] `sessions/`: move `drivers.rs`, `session/` (`mod.rs`, `state.rs`, `model.rs`, `subagent.rs`, `launch/`), `restore.rs`, `takeover.rs`, `foreground.rs`, `memory.rs`, `oom.rs`, and `context.rs` into `src/sessions/`. Move `SessionError` from `error.rs` into `sessions/error.rs` (`storage/sessions.rs` stays in `storage/` while `storage/canvas.rs` and `storage/efforts.rs` still share its helpers).
- [x] `mcp/`: move `socket.rs`, `tools.rs`, and `tools/` (`read.rs`, `map.rs`, `research.rs`) into `src/mcp/`.
- [x] Switch `dovetail_api` imports in moved files to full domain paths. Keep temporary `lib.rs` re-exports (`drivers`, `memory`, `session`, `socket`, `takeover`, etc.).

### Step 5 — Move `visual/`, `research/`, and `activity/`
- [ ] `visual/`: move `canvas.rs`, `design.rs`, `preview.rs`, `mcp/tools/map.rs`, and SQL (`canvas.rs`, `design.rs`, `maps.rs` from `storage/`) into `src/visual/`.
- [ ] `research/`: move `research/` (`mod.rs`, `capture.rs`, `files.rs`, `report.rs`), `ask.rs`, `browse.rs`, `docs.rs`, `mcp/tools/research.rs`, and `storage/research.rs` into `src/research/`.
- [ ] `activity/`: move `inbox.rs`, `usage.rs`, and extract feed/warning/totals helpers from `service.rs` into `src/activity/`.
- [ ] Switch `dovetail_api` imports in moved files to full domain paths.

### Step 6 — Move `tracking/` and `agents/`
- [ ] `tracking/`: move `projects.rs`, `project_admin.rs`, `registration.rs`, `home.rs`, `efforts.rs`, and SQL (`efforts.rs`, `tasks.rs` from `storage/`) into `src/tracking/`. Move `RegisterError` from `error.rs` into `tracking/registration.rs`.
- [ ] `agents/`: move `slice.rs`, `tangents.rs`, `journal/` (`mod.rs`, `switch.rs`), `todos.rs`, `attachments.rs`, `completion.rs`, `side_chat.rs`, `report.rs`, `agy_models.rs` into `src/agents/`. Move `SliceError` and `SetupFailure` from `error.rs` into `agents/slice.rs`, and dissolve `src/error.rs`. Tidy manual `From` impls in `tangents.rs` where `#[from]` fits.
- [ ] Switch `dovetail_api` imports in moved files to full domain paths.

### Step 7 — `engine/`, `Core` state structs, thin routers, and import cleanup
- [ ] Move `service.rs`, `shutdown.rs`, `startup.rs` into `src/engine/`.
- [ ] Introduce per-domain state structs (`TrackingState`, `AgentState`, `ActivityState`, `VisualState`, `ResearchState`, `CodeState`, `AppState`, `SessionState`, `McpState`) on `Core`.
- [ ] Split `Core::command`, `Core::tool`, and `Core::on_driver` into thin routers that delegate to domain handlers (`tracking`, `agents`, `activity`, `visual`, `research`, `code`, `app`, `sessions`).
- [ ] Verify every `dovetail_api` import in `crates/dovetail-core/src/` uses full domain paths (D43).
- [ ] Update `crates/dovetail` and `crates/dovetail-core/tests/` to use the new `dovetail_core::<domain>::...` paths if desired, or keep intentional root re-exports documented in `lib.rs` (decided at Step 7).

### Step 8 — Split `tests/sessions.rs` by domain + `ARCHITECTURE.md`
- [ ] Split `crates/dovetail-core/tests/sessions.rs` into one integration test file per domain (`tests/tracking.rs`, `tests/agents.rs`, `tests/activity.rs`, `tests/visual.rs`, `tests/code.rs`, `tests/app.rs`, `tests/sessions.rs` for process lifecycle/restore/takeover/caps), all using `tests/common/mod.rs`.
- [ ] Write `crates/dovetail-core/ARCHITECTURE.md` documenting the infrastructure/domain map, `Core` loop, state structs, storage thread, and how to add a command/tool/domain.
- [ ] Include it in `crates/dovetail-core/src/lib.rs` via `#![doc = include_str!("../ARCHITECTURE.md")]`.

## Risks and open questions

- **Migration path drift during rebase with `main` (Step 3):** `scripts/check.sh` checks `crates/dovetail-core/src/db/migrations` against `origin/main`. Moving `db/migrations/` to `storage/migrations/` must update `scripts/check.sh` in the same commit, and while `origin/main` still has `src/db/migrations/`, `check.sh`'s `git ls-tree origin/main` comparison will need to look at both `src/db/migrations` and `src/storage/migrations` (or fall back to `src/db/migrations` on `origin/main`) so parallel migration-clash detection keeps working.
- **Cross-domain `Core` methods:** several handlers touch two domains (for example, archiving a slice updates `agents`, `tracking` tasks, and `code` worktrees; tangents touch `agents` and `research`). Keeping `impl Core` methods in the owning domain module (rather than trying to pass `&mut DomainA, &mut DomainB` everywhere) avoids borrow-checker friction while still grouping state cleanly into domain structs on `Core`.
- **SQL placement vs. `Db` thread:** `Db` runs on one SQLite thread with a typed `DbReq` enum. Moving domain SQL functions (`db/tasks.rs`, `db/research.rs`, etc.) into each domain means `storage/mod.rs` dispatches `DbReq` variants to `crate::<domain>::db::*` functions (taking `&mut Connection`), keeping one DB thread without moving domain logic into `storage/`.
- **Public API re-exports:** `crates/dovetail` imports `config`, `paths`, `profile`, `recipe`, `upgrade`, `worktree`, `projects`, `socket`, `db`, `service`. Temporary `pub use` re-exports in `lib.rs` keep `crates/dovetail` untouched during Steps 2–6; Step 7 decides which re-exports stay permanent vs. migrate to domain paths.
