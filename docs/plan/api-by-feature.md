# Plan: dovetail-api by feature (#54), then agent tools in their own crate (#55)

Status: **paused part-way** (2026-11-21, the weekly usage limit). This file is meant to be enough for
someone with none of the earlier context to finish both tasks. The work in progress is on the branch
`restructure-wip`; the destination branch is `restructure` (never `main`).

## 1. Goal

- **#54** Restructure `crates/dovetail-api` by feature: one folder per feature, each with its types
  (`mod.rs`), its `Command` sub-enum (`command.rs`), its `Event` sub-enum (`event.rs`) and its reducer
  (`apply.rs`). The top-level `Command` and `Event` become enums with one variant per feature, so a
  caller writes `Command::Task(TaskCommand::Create(..))` instead of `Command::CreateTask(..)`.
  The goal is readability for a human maintainer, who is reviewing the code crate by crate; this is the
  first crate.
- **#55** After #54 is whole and green: move `tools.rs` and `socket.rs` (the agent-to-core protocol) into
  a new crate `crates/dovetail-tools`, organised by feature in the same way. The TUI must not depend on it.
- Decisions that bind this work: **D36** (on #47: the restructure itself, the target layout) and
  **D37** (the branch rule): all work goes on the long-lived branch `restructure`, created from `main`;
  merge `origin/main` into it (never rebase), push fast-forward only with
  `git push origin HEAD:restructure`, never force-push, and nothing goes to `main` until the user says.
  Each of #54 and #55 is pushed whole and green, as its own push, #54 first.
- Don't change behaviour. The serde JSON of `Command` and `Event` does change (nested enums:
  `{"task":{"create":{..}}}`). That is accepted: only the core and the TUI consume it and they ship together.
  Report it.

## 2. Target layout of `crates/dovetail-api/src`

```text
lib.rs            module list and the few root re-exports (Command, Event, ServerMsg, CoreLink,
                  PROTOCOL_VERSION, Snapshot, apply, ApplyResult); #![doc = include_str!("../ARCHITECTURE.md")]
snapshot.rs       Snapshot (flat fields, as before) + trimmed() + the tail caps
                  (ACTIVITY_TAIL, MESSAGE_TAIL, CONVERSATION_TAIL, CONVERSATION_KEEP)
wire.rs           ServerMsg, CoreLink, PROTOCOL_VERSION, and the top-level enums Command and Event
apply.rs          ApplyResult, apply() (seq/gap/stale, then one arm per feature), shared helpers upsert, push_bounded
<feature>/
  mod.rs          the feature's types; `mod command; mod event; mod apply;` and explicit `pub use` of
                  the sub-enums and payload types (no glob re-exports)
  command.rs      <Feature>Command, plus payload types only that command uses
  event.rs        <Feature>Event
  apply.rs        pub(crate) fn apply(state: &mut Snapshot, event: <Feature>Event)
tools.rs, socket.rs   stay here until #55
```

`ids.rs` was **not** created: every id type (`TaskId`, `AgentId`, `InboxId`, …) belongs to one feature and lives
in it (`ProjectId` and `RepoId` in `project`). Create `ids.rs` only if a shared id turns up.

Snapshot keeps flat fields (`state.tasks`, `state.agents`, …). Nesting it per feature would touch every UI read for no gain.

### Features (24 folders)

`Command` / `Event` variants are given as *old name → new name*. The authoritative table is
`scripts/restructure-api/spec.py` (`CMD`, `EVT`, `ITEM_DEST`).

| Feature | Types (mod.rs unless noted) | Commands | Events |
|---|---|---|---|
| `task` | `TaskId`, `Task`, `TaskStatus`, `PLevel`, `Bucket`; command.rs: `NewTask`, `TaskChange` (+`describe`), `describe_task_patch` | CreateTask→Create, UpdateTask→Update, MoveTask→Move, ArchiveTask→Archive, ReorderTask→Reorder | TaskUpserted→Upserted, TaskRemoved→Removed |
| `decision` | `DecisionId`, `Decision`, `Alternative`, `DecisionAuthor`; command.rs: `NewDecision` | LogDecision→Log | DecisionAdded→Added |
| `agent` | `AgentId`, `Agent`, `RunningTool`, `AgentKind`, `WorktreeRef`, `AgentStatus`, `PermissionMode`, `SubagentProgress`, `SessionLock`, `SlashCommand`, `TodoItem`, `TodoStatus`, `TodoListId` | Interrupt, CyclePermissionMode, NewSlice, CloseSlice, NewSideChat | AgentUpserted→Upserted, AgentRemoved→Removed, CommandsChanged, TodosReplaced |
| `conversation` | `EntryId`, `ConversationTail`, `ConversationEntry`, `EntryKind`, `AgentMessageParts`, `agent_message_text`; attachment.rs: `Attachment`, `ImageType`, `MAX_ATTACHMENTS`, `MAX_ATTACHMENT_BYTES`, `size_text`, `attachment_limit_text` | Prompt, LoadConversation→Load, ListPaths | ConversationAppended→Appended, AssistantDelta, ConversationPage→Page, PathsListed |
| `inbox` | `InboxId`, `InboxItem`, `InboxKind`, `RuleSuggestion`, the four `*_PERMISSION` constants; command.rs: `PermissionAnswer` | AnswerPermission, AnswerQuestion, DismissInbox→Dismiss | InboxUpserted→Upserted, InboxResolved→Resolved |
| `model` | `ReasoningEffort`, `BUILTIN_MODELS`, `ModelOption`, `model_option_index`, `DriverKind` (from the old `driver.rs`) | SetModel→Set | ModelsChanged→Changed |
| `mcp` | the old `mcp.rs` unchanged | Mcp→Request | McpReply→Reply |
| `journal` | `JournalState` | EndOfDay, Handoff, FreshSession | JournalChanged→Changed |
| `tangent` | `TangentSummary`, `tangent_agent`, `tangent_task`, `TANGENTS_PROJECT` | NewTangent→New, AttachTangent→Attach, FileTangent→File, StartFromTangent→StartFrom, DropTangent→Drop, LoadTangent→Load | TangentUpserted→Upserted, TangentRemoved→Removed, TangentLoaded→Loaded |
| `terminal` | none | OpenEditor, TerminalReleased→Released, TakeOver | LendTerminal→Lend, TerminalReturned→Returned |
| `clipboard` | `Paste` | PasteClipboard→Paste, PasteText, CopyToClipboard→Copy | ClipboardPasted→Pasted |
| `upgrade` | none | Upgrade→Start | UpgradeBuilding→Building, UpgradeRestarting→Restarting |
| `canvas` | the old `canvas.rs` (incl. `pub mod geometry`) | CreateBlock, RenameBlock, AddCard, PlaceCard, SeeCard, MoveCard, DeleteCard, DropIncoming, LinkCards, UnlinkCards, CardFromReply, LoadCard | BlockUpserted, CardUpserted, CardRemoved, CardLinked, CardUnlinked, CardLoaded |
| `design` | the old `design.rs` | StartPreview, RestartPreview, SelectVariant, KeepVariants | PreviewChanged, VariantsUpserted |
| `effort` | the old `effort.rs` | CreateEffort→Create, RenameEffort→Rename, UpdateEffort→Update, SetWhereWasI, AddCriterion, ClaimCriterion, ApproveCriterion, ReturnCriterion | EffortUpserted→Upserted |
| `worktree` | `WorktreeInfo`, `SyncState`, `Changes`, `PrLookup`, `PullRequest`, `PrState`, `WorktreeSetup` | RemoveWorktree→Remove, RefreshWorktrees→Refresh | WorktreeUpserted→Upserted |
| `project` | `ProjectId`, `RepoId`, `ProjectSummary`, `RepoSummary`, `ProjectCounts`, `ProjectKind` | OpenProject→Open, NewResearchProject→NewResearch, CreateProject→Create, RenameProject→Rename, SetProjectAccount→SetAccount, AddRepo, RemoveRepo, RegisterProject→Register, SetRecipeProfile | OpeningProject→Opening, ProjectUpserted→Upserted, RecipeStarted |
| `map` | the old `map.rs` | LoadMap→Load | MapUpserted→Upserted, MapLoaded→Loaded |
| `research` | the old `research.rs` minus the library items | StartRound, CloseRound, DropRound, SetResearchKind→SetKind, FileUnfiled, LoadResearch→Load, StartAsk | ResearchUpserted→Upserted, ResearchLoaded→Loaded |
| `library` | `DocEntry`, `DocImage`, `BrowsedProject`, `BrowsedListing`, `BrowseWhat`, `BrowsedDoc` (taken out of `research.rs`) | ListDocs, LoadDoc, BrowseProject, LoadBrowsed | DocsListed, DocLoaded, ProjectBrowsed, BrowsedLoaded |
| `app` | none | OpenExternal, Resync, Quit | CommandFailed |
| `activity` | `ActivityItem`, `ActivityKind`, `AgentMessage` | none | ActivityAdded→Added, MessageAdded |
| `status` | `Totals`, `Warning`, `WarningId`, `UNREGISTERED_WARNING` | none | TotalsChanged, WarningSet, WarningCleared |
| `usage` | `UsageLimits`, `UsageWindow`, `UsageStatus` | none | UsageChanged→Changed |

Features with no commands have no `command.rs`; features whose events change no state (`library`, `mcp`,
`app`, `clipboard`, `terminal`) have no `apply.rs`, and `apply.rs` at the top lists them in one no-op arm.
The agy driver slice (#46) adds usage and per-driver model types to the api: they go into `usage/` and `model/`.

### Choices made, to report with the push

- **`Bucket` and `BucketFilter` are not dead**: `Bucket` is used by the task model, the core db, the journal and the TUI;
  `BucketFilter` by the `core_list_tasks` tool. Both stay (`BucketFilter` moves to `dovetail-tools` in #55).
- **Helpers that move**: only `ImageType::sniff` (used only in `dovetail-core`: `attachments.rs`, `clipboard.rs`) and
  `VariantSet::keep_prompt` (used only in `dovetail-core/src/design.rs`) move to the core, as plain functions next to their callers.
- **Helpers that stay in their feature folder** because they have two or more users: `Evidence::parse` (the type's own
  deserialisation and the TUI), `describe_task_patch` / `TaskChange::describe` (core and mock), `agent_message_text`
  (core, TUI; it pairs with `ConversationEntry::agent_message`), `size_text` (core and TUI), `attachment_limit_text` (kept with its constant).
- **GUI leftovers to remove by hand**: wording in `canvas/mod.rs` (`geometry` doc: "the GUI draws with"), `design/mod.rs`, and
  `ProjectSummary` (`repo_path`/`kind` docs: "for the frozen TUI … the GUI reads `repos`").
- **Exposure rule**: features are `pub mod`; each `mod.rs` re-exports its sub-enums and payload types explicitly; no glob
  re-exports anywhere; the crate root re-exports only the names in the layout above.

## 3. What is done and what is on `restructure-wip`

Base: `main` at `9727ac8` (agy driver over ACP). The branch is **one WIP commit on top, that does not compile**.

Done (committed, never compiled):
- `crates/dovetail-api/src/<feature>/` for all 24 features, generated by the scripts in `scripts/restructure-api/` from the old
  `model.rs`, `protocol.rs`, `driver.rs` and the whole-file modules: `mod.rs`, `command.rs`, `event.rs` with imports, sub-enums
  and renamed variants, doc links rewritten, plus `apply.rs` per feature (written by hand from the old `apply.rs`).
- `snapshot.rs` (Snapshot, `trimmed`, tail caps), `wire.rs` (ServerMsg, CoreLink, and the nested `Command` and `Event`), `apply.rs`, `lib.rs`.
- The old `model.rs`, `protocol.rs`, `driver.rs`, `effort.rs`, `canvas.rs`, `design.rs`, `map.rs`, `mcp.rs`, `research.rs` are deleted.
- `ARCHITECTURE.md` is a **one-line stub** (lib.rs includes it, so the crate doc builds).

Not done:
- The new tree has **never been through `cargo build`**: expect missing and unused imports, doc links to fix, and name clashes.
- `tools.rs` and `socket.rs` still import `crate::{AgentId, NewTask, ..}`: they need the new paths.
- No caller is updated (core, TUI, mock, maps, binary). `rewrite.py` is a first draft that has **not been run** (see section 5).
- Tests are untouched and still use the old API.
- The two helper moves, the leftover removal, the event-coverage test and the real `ARCHITECTURE.md`.

## 4. The scripts (`scripts/restructure-api/`)

Throwaway tooling, kept so the split can be audited or redone. They need Python 3 and no packages.

- `spec.py`: the tables. `ITEM_DEST` (type → file), `CMD` and `EVT` (old variant → feature and new name), `WHOLE`, `LIBRARY_ITEMS`.
- `generate.py SRC OUT`: reads the **old** layout from `SRC` (a folder holding `model.rs protocol.rs driver.rs effort.rs canvas.rs design.rs map.rs mcp.rs research.rs`)
  and writes the feature folders, `snapshot.rs` and `wire.rs` (types only) into `OUT`. Get the old files with
  `for f in model protocol driver effort canvas design map mcp research; do git show 9727ac8:crates/dovetail-api/src/$f.rs > /tmp/old/$f.rs; done`.
  Not needed again: its output is what is committed. Rerun only to redo the split from scratch (it overwrites hand edits).
- `modlines.py SRC`: adds the `mod command; mod event; mod apply;` and `pub use` lines to each feature's `mod.rs`, from the files that exist.
- `wire.py` (run from `src/`): adds the top-level `Command`/`Event` enums to `wire.rs` and writes the top-level `apply.rs`.
- `rewrite.py PREFIX API_SRC FILE...`: rewrites **callers**: `use dovetail_api::{..}` lists and `dovetail_api::Name` paths to the feature
  modules, and `Command::Old` / `Event::Old` (with brace matching) to `Command::Feature(FeatureCommand::New(..))`. `PREFIX` is `dovetail_api` for other
  crates, `crate` inside the api. Known gaps in the draft: tail-cap names (`CONVERSATION_TAIL`, …) have no public path yet (see step 2);
  a variant used as a function value (`.map(Event::TaskUpserted)`) is not handled; where a caller also imports a different `TaskEvent`
  (the driver has one), the import must be renamed by hand. Run it on a copy first (`git stash` is shared, so use a WIP commit to set work aside), read the diff.

The per-feature `apply.rs` files were written by hand (not by a script); do not regenerate them.

## 5. Remaining steps for #54, in order

Work in the worktree on the branch `dovetail/api-by-feature`. Check each step with `cargo build -p dovetail-api` (steps 1-2) and
`CARGO_BUILD_JOBS=2 scripts/check.sh` (the end). Commit after each step.

1. **Make `dovetail-api` build.** Files: `crates/dovetail-api/src/**`. `cargo build -p dovetail-api`; fix imports (`cargo fix` is unsafe here, it removes
   imports that only `#[cfg(test)]` code uses; remove unused ones by hand), tidy each `mod.rs` header (they read "The `task` feature: tasks: …", drop the
   doubled word), and fix doc links with `cargo doc -p dovetail-api --no-deps` (links to `Command::X` already became `FeatureCommand::New`; names used only in
   docs need a full `crate::` path). Update `tools.rs` and `socket.rs` imports. Decide where the tail caps are public: make `snapshot` a `pub mod` and
   have callers write `dovetail_api::snapshot::CONVERSATION_TAIL`, or re-export the four names from lib.rs. Verify: the api library builds, with no warnings.
2. **Callers.** Files: `crates/dovetail-core`, `dovetail-tui`, `dovetail-mock`, `dovetail-maps`, `dovetail` (about 1,200 `Command::`/`Event::` uses and 200 imports).
   `python3 scripts/restructure-api/rewrite.py dovetail_api crates/dovetail-api/src $(git ls-files 'crates/*.rs' ':!crates/dovetail-api/**')`, then `cargo build --all-targets`
   and fix what is left by hand (variants used as values, clashes with `dovetail_driver::TaskEvent`, `use dovetail_api::*;` in 24 test files, which the script expands to the names
   the file uses). Run `cargo fmt`. Verify: `cargo build --all-targets` is clean.
3. **Helper moves and leftovers.** Move `ImageType::sniff` into `dovetail-core/src/attachments.rs` (a function `sniff(bytes) -> Option<ImageType>`; update `clipboard.rs`), and
   `VariantSet::keep_prompt` into `dovetail-core/src/design.rs`. Remove the GUI wording listed in section 2. Check `docs/` and the code for other "GUI" mentions in the api.
4. **Tests: one file per feature.** Replace `crates/dovetail-api/tests/{apply.rs,apply_cases.rs,serde_roundtrip.rs}` with one integration test binary `tests/features/main.rs`
   and `tests/features/<feature>.rs`. Each feature file holds that feature's serde round-trips and reducer tests, and exposes `pub fn cases() -> Vec<Case>` (the `Case`
   struct, builders such as `task()` and `agent()`, and `result_name` go in `tests/features/common.rs`). `main.rs` also has the seq/gap/stale tests (`wire.rs`/`apply.rs`) and the old
   "every case replays" test. The round-trip tests carry literal JSON with the old variant names: rewrite them to the nested form (`{"task":{"upserted":..}}`); the new JSON is what the changed
   code serialises, so regenerate expected strings from a failing run and read them before accepting.
5. **The every-Event-has-a-case check** (it was lost with #52 Shelve TypeScript page and generation). In `tests/features/coverage.rs`: read each `src/*/event.rs` with `std::fs`, find `pub enum <X>Event {`, take the
   variant names at four-space indent, and turn each into the key `feature.variant_in_snake_case`. Collect the events of every feature's `cases()`, serialise each with `serde_json::to_value`, and derive the same key
   (top key plus the inner string or sole key). Assert that the two sets match, listing the missing kinds. A new variant with no case then fails the test. Events that change nothing (loaded, listed) still need a case that checks the
   state stays equal apart from `seq`.
6. **`crates/dovetail-api/ARCHITECTURE.md`**, plain words, in the style of `crates/dovetail-driver/ARCHITECTURE.md`: purpose, the two directions (`Snapshot` then `Event`s out,
   `Command`s in), the layout, how a snapshot is kept up to date (`apply`), how to add a feature (new folder, `Command`/`Event` variant in `wire.rs`, a line in `apply.rs`, a test file, a case per event) and how to add a variant to an existing feature.
   Keep the `#![doc = include_str!("../ARCHITECTURE.md")]` line in lib.rs; remove the stub.
7. **Merge main, check, push.** `git fetch origin && git merge origin/main` (the agy slice keeps editing `dovetail-api`; a modify/delete conflict on `model.rs`, `protocol.rs` or `driver.rs` means they added a type: put it in the feature
   folder it belongs to, usage and per-driver model types in `usage/` and `model/`, and new `Command`/`Event` variants into the right sub-enum with a case in the test). Then `CARGO_BUILD_JOBS=2 scripts/check.sh`;
   it must end on its pass line. Push with `git push origin HEAD:restructure` (fast-forward only, never force). Report: the commit, the final tree with line counts (`find crates/dovetail-api -name '*.rs' | xargs wc -l`),
   the wire-format change, and what was left on purpose (section 2).

## 6. Steps for #55: `dovetail-tools`, after #54 is pushed and green

Same branch rules. Do it as its own push.

1. New crate `crates/dovetail-tools` (workspace member, `Cargo.toml` like the api's, depends on `dovetail-api` and `serde`/`serde_json`; no other dependencies). Move `tools.rs` (the MCP tool catalogue, roles and suites,
   validation, tool inputs, `BucketFilter`) and `socket.rs` (the unix-socket messages) out of `dovetail-api`, organised by feature like the api: one folder per feature (`task`, `canvas`, `design`, `research`, `report`, `project`, …),
   each with its tool inputs and validation, and a small top-level `Tool` enum with one variant per feature. Pick the folders from the current `Tool` variants and input structs, and say what you chose. Keep `Role`, `Suite`, `ToolSpec`, `TOOLS`,
   `ToolError` and the `env` constants in their own files.
2. Update the imports in `dovetail-core` (`tools/`, `socket`, the MCP server), the `dovetail` binary (`dovetail mcp`, `dovetail hook`) and `dovetail-mock`. **`dovetail-tui` must not depend on it**: check with `cargo tree -p dovetail-tui | grep dovetail-tools` (empty).
   Tests that exercised tools in the api move with them.
3. `crates/dovetail-tools/ARCHITECTURE.md`, included into its `lib.rs` as for the api. Update `AGENTS.md` and `docs/architecture.md` where they list the crates (a "tools" crate between the api and the core; and the rule that the TUI does not depend on it).
4. Merge main, `CARGO_BUILD_JOBS=2 scripts/check.sh`, `git push origin HEAD:restructure`, report.

## 7. Rules and risks

- Never push to `main`; never force-push `restructure`; merge, don't rebase. Commit messages end with `(task #54)` or `(task #55)` and the attribution line the harness gives.
- `agy-driver-2` (#46) keeps landing on `main` and touches `dovetail-api` (`DriverKind`, usage and model types), `dovetail-core`, `dovetail-tui`. Merge `origin/main` before each push and after step 2 (the biggest diff). Do not edit `dovetail-driver` except its imports.
- Callers use the old names in new code that lands on main meanwhile; after a merge, run `rewrite.py` on just the files main changed, then build.
- The core and the TUI are only consumers of the changed JSON, but check that nothing stores serialized `Command` or `Event` values (the db, fixtures): `grep -rn "serde_json::to_string\|from_str" crates/dovetail-core/src | grep -i "event\|command"`.
- Real sessions cost money: do not run Claude Code against real projects to test this; the mock core and `cargo test` cover it.
- Done means `CARGO_BUILD_JOBS=2 scripts/check.sh` passes.
