# v2 stack review

Review of the v2 stack, PRs #75, #77 and #78 (#78 contains #76's frontend). The diff ran from `origin/main` to `claude/v2-stack-review`: that branch is `claude/tauri-shell` with current `main` merged in. It is about 20,000 lines; 4,700 of them are in `crates/`, `scripts/` and `gui/src-tauri`, and the rest is the React frontend. The review used `/code-review`, with two Opus reviewers, Standards and Spec. Unattended run, 2026-11-16.

Standards: `CODING_STANDARDS.md`, plus the hard constraints and architecture rules in `AGENTS.md`. The ratatui and "any terminal" rules do not apply to `gui/`. Spec: `docs/design-v2.md`, ADRs 0021–0025 and `docs/gui-layout-and-keys.md`.

**Result:** after the fixes, `scripts/check.sh` passes (fmt, clippy, every test, the generated types current, and the GUI's type check, tests and build). The app starts against scratch directories. `HOME` and every XDG directory pointed at a scratch folder, so no installed `dovetail` binary was found and no session could start. The Home core ran, and the window loaded from the dev server and subscribed. No real Claude Code session was run.

## Checked and holding

- **Migration 0017** only adds things: two tables, an index, the Unsorted row (id 1) and three nullable columns. Its one `UPDATE` fills the new `tasks.effort_id`. Tests cover a migration from empty and one up from version 16; the second keeps existing rows, and opening again changes nothing.
- **One loop, one DB thread.** No rusqlite outside `dovetail-core/src/db*`, and none in the Tauri shell. The new handlers (`efforts.rs`, `report.rs`, `follow_pull_request`) run on the core loop through `self.db`. Home is a second, read-only loop.
- **No agent can mark a criterion met or set task status.** `tasks_update` refuses `status`, and the field is gone from its schema. Criteria change only through user `Command`s. `coordination_report` from anything but a slice is refused.
- **The Tauri shell passes `ServerMsg` and `Command` through untouched.** It only looks at `Event::OpeningProject` to know when a core stops.
- **The generated TypeScript types are current.** Regenerating them changes nothing.
- **The frozen TUI** builds and passes. It shows nothing new: Review maps back to In progress, and no effort is shown.

## Standards

| Finding | Kind | Outcome |
| --- | --- | --- |
| `gui/src-tauri/Cargo.toml` declared `serde_json` but never used it | AGENTS.md "How to work" 6 | **Fixed** |
| The shell's `main` used `.expect(...)` for the supervisor thread and the Tauri build: without WebKitGTK, launching panicked | CODING_STANDARDS.md "Code": no `expect` outside tests; `anyhow` at the binary edge | **Fixed**: `main` returns `anyhow::Result` |
| `pr_stage` gave PR progress as a bare `u8`, matched as `0/1/2` | Primitive Obsession, judgement | **Fixed**: ordered `PrStage` enum, no behaviour change |
| `spawn_home` reported a failed thread spawn as `DbError::Io { path: "dovetail-home" }` | Mysterious Name, judgement | **Fixed**: `CoreError::Thread` |
| `send_input` → `send_input_tagged` and `add_entry_with` → `add_entry_tagged` are pass-through pairs; a 6-argument clump travels between them | Middle Man / Data Clumps, judgement | Open: fold them if more tag sources appear |

## Spec

| Finding | Kind | Outcome |
| --- | --- | --- |
| Session names went stale after `:project add`. A coordinator launched as `new-<pid>.coordinator` kept that name, but `session_name`/`bare_name` switched to the slug. Slices were then told the wrong coordinator name, and the coordinator's messages came back unstripped as a phantom agent | ADR 0022: "the core strips the prefix where it reads a name back" | **Fixed**: the prefix is fixed for the process (`Core::session_prefix`). New test `sessions_keep_their_launch_prefix_after_project_add` failed before the fix |
| The shared apply cases covered 11 events, with nothing to enforce more, and `apply.ts` ended in a silent `default` | ADR 0023 point 5: "a new `Event` needs a new case; both sides fail without one" | **Fixed**: a fourth case, `the_rest`, covers every event the reducer stores. A Rust test fails when a tag in the generated `Event.ts` has no case and is not listed as passed through. The `apply.ts` switch is exhaustive, so `tsc` names any event it does not handle |
| `GLOSSARY.md` said Coordination is "empty until v1.1 Claims" | Stale doc | **Fixed** |
| ADR 0022's Consequences still name `core_report` | Stale doc | No change: its "As built" section records `coordination_report`. ADRs keep the text that was decided |
| "Send it back" does not stick. When the user sends a Review task back while its PR stays open, `follow_pull_request` moves it to Review again after the slice's next turn | ADR 0024 lists both rows and does not say which wins | **Decision for you** |
| `ApproveCriterion` works on an open criterion (`open → met`, with no claim first) | design-v2: "agent claims … ► user approves". Still user-only | **Decision for you**: refuse it, or keep it as the user's shortcut |
| `ClaimCriterion` exists only as a user `Command`; no agent tool claims a criterion | design-v2 says agents claim | **Decision for you**: build the agent tool now (which suite), or leave it for later |

## Also done (agreed by the user)

- The GUI dev server moved from port 5174 to **47815**. `gui/vite.config.ts` and `tauri.conf.json` hold the same value, and `scripts/check.sh` fails if they differ.
- `dovetail-gui` stays a workspace member but is not in `default-members`, so a plain `cargo build`/`cargo test` does not need WebKitGTK. `scripts/check.sh` still builds, lints and tests it through `--workspace`.

## Questions for you

1. When you send a task back from Review while its PR is still open, should it stay In progress until something new happens (a new PR state, or the slice reporting again)?
2. Should approving a criterion that was never claimed be refused?
3. Should agents get a tool to claim a criterion now, and in which suite?
