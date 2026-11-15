# AGENTS.md — Dovetail

Instructions for any coding agent working in this repository. Read in this order:

1. this file;
2. `docs/design-digest.md` — the implementation-relevant design: locked decisions, spike results, protocol facts, roadmap, open spikes;
3. `docs/design.md` — the full design doc (when present; it wins over the digest);
4. `docs/adr/` — decisions made after the v1 baseline (when present; an ADR wins over the design doc and digest on the decision it names);
5. `docs/ui-session-brief.md` — detailed UI spec: data model, keys, readability, switcher, quick capture;
6. `docs/screens/` — mock screens from the design phase (`README.md` there says which are v1 targets, which are superseded); look at `ui/b.png` and `ui/navigation.png` first. Screens are reference, the docs are spec;
7. `README.md` — product overview.

If an implementation plan exists (`docs/implementation-plan.md`), it is the current source of truth for *order of work*. Keep it updated as milestones finish.

## What this is

Dovetail is a keyboard-driven terminal UI, written in Rust, for running many Claude Code agents on one feature: a **coordinator** session the user works with directly, long-lived **slice** sessions (one git worktree each), and the **subagents** they spawn. It tracks the user's tasks, shows every agent under its task, and brings anything that needs the user into one inbox. Sessions run **headless** over Claude Code's NDJSON stream protocol; Dovetail serves its own tools to them over MCP (`dovetail mcp`).

The design phase is finished. **All decisions are locked.** Do not reopen them; if something looks wrong, contradictory or under-specified, stop and ask.

## Scope

The target is **v1** as listed in the digest's roadmap, built so v1.1 features slot in without rewrites. v1.1+ features are out of scope unless the plan says otherwise, but leave the seams for them (e.g. the keymap table must be able to become TOML).

Known conflicts, already resolved:
- **Layout:** Decision 15 (Layout B — conversation full height on the left, task tree over inbox on the right; placement per ADR 0006) supersedes the sketch in `ui-session-brief.md`.
- **Clients:** ADR 0020 makes an egui GUI the main client and cuts the TUI to a remote kit (chat, inbox, tree, switcher); `ratatui` + `crossterm` now bind only the TUI.
- **Edition:** the workspace uses edition 2024 (already in `Cargo.toml`).

## Dogfooding: Dovetail builds Dovetail

The user wants to switch from building Dovetail with plain agents to building it **inside Dovetail** as early as possible.

- **Self-hosting point.** The plan must name the earliest milestone after which the rest of the build can run inside Dovetail, and order the work to reach it fast. Minimum: real headless coordinator session with streaming chat, permission requests answered from the inbox, slice sessions in hook-created worktrees, `core_report_status`/`core_ask_user`, and state that survives a restart. Polish (board, maps, memory caps, take-over, context-fill display) comes after it.
- **From that point on**, the repo's own role prompts and recipe are used for real, so they must stay accurate. Sources of truth:
  - `config/roles/coordinator.md`, `config/roles/slice.md` — role prompts (keep each under ~20 lines, Decision 8c). Dovetail reads the installed copies in `~/.config/dovetail/roles/`.
  - `config/projects/dovetail.toml` — the worktree recipe for this repo (format is a proposal; the plan finalises it and the core must read it).
  - `scripts/bootstrap.sh` — creates Dovetail's dirs, installs the prompts and recipe without overwriting user edits, and checks prerequisites (Claude Code ≥ 2.1.224, systemd-run caps, messaging blockers). Keep it in step with any path or format change.
- Self-hosting must never weaken the rules: slices working on Dovetail still get their own worktree, and nothing is written into the repo at runtime beyond branches.

## How to work

1. **Plan before code.** Start by producing `docs/implementation-plan.md` (see "The plan" below) and wait for the user's explicit OK.
2. **Spikes before the code that depends on them.** Each open spike in the digest gates specific work. Run it as a small throwaway program under `spikes/<name>/` (not a workspace member), record the result in `docs/spikes/<name>.md`, and report it. Spikes that need the user's machine, account or judgement (terms of use, cgroups on their box) are prepared as runnable scripts and handed to the user.
3. **One wave at a time.** A wave is a set of milestones named in the plan (§6, "Waves"). Agents build a wave's tracks in parallel; at the end of the wave an Opus `code-review` runs from the last review tag, the user runs `retro`, and the wave merges. Milestone "done when" boxes are still ticked one by one. A wave is done when `scripts/check.sh` passes (fmt, clippy, tests and mechanical rule checks), except where the plan relaxes it for that wave (wave 1: `cargo build` only, no new tests; decided 2026-11-05). Then summarise what changed and how to try it.
4. Keep changes scoped to the milestone. No speculative features, no drive-by refactors.
5. User-facing decisions (UX, keys, visible naming) → ask. Implementation details with sensible defaults → decide, note in the summary.
6. Never add a dependency without stating why. Prefer the standard library and small crates.
7. **Never run real Claude Code sessions against the user's real projects** during development. Use a scratch git repo under the target dir or `/tmp`. Real sessions cost money: keep integration tests behind an opt-in flag (`DOVETAIL_LIVE_TESTS=1`) with tiny prompts.

## Conventions for skills

- **Standards:** `CODING_STANDARDS.md`, plus the hard constraints and architecture rules in `AGENTS.md`. **Spec:** `docs/implementation-plan.md` plus `docs/design-digest.md`.
- **Decisions:** the decision log in `docs/design.md` is the frozen v1 baseline, never edited. New decisions that meet the `domain-modeling` ADR bar go in `docs/adr/NNNN-slug.md`, linking the spike or discussion behind them. An ADR may refine or supersede a locked decision only with the user's approval in the session, and must name the decision it changes.
- **Terms:** `GLOSSARY.md` at the repo root.
- **Spikes:** code in `spikes/<name>/`, results in `docs/spikes/`, never `prototype/` branches.
- **Commits:** the agent commits, merges to `main` and pushes on its own judgement (decided 2026-11-05).

Which skill when:

- **Reviews:** `code-review`, then `retro`.
- **Tests:** written when something breaks, from wave 2 on; wave 1 adds none (decided 2026-11-05).
- **Failures:** `diagnosing-bugs`.
- **S0 and S1:** `research`.
- **Context running low:** `handoff`.
- **Slice PRs:** the convention in `config/roles/slice.md`.

`retro` and `handoff` are user-invoked: ask the user to run them.

## Hard constraints

- **Rust** stable, `ratatui` + `crossterm`, `nucleo` (fuzzy), `pulldown-cmark` (markdown), `rusqlite` with `bundled` (SQLite, FTS5, WAL) on a dedicated thread, `tokio` for process/IO in the core only.
- **Low memory:** 16 GB target machine, most of it for Claude Code. Lean dependencies. The TUI **redraws only when state changed** (dirty flag, streaming coalesced to ≤ ~30 fps, no idle ticks).
- **Any terminal.** kitty is the default but never rely on the kitty keyboard protocol. kitty graphics only for the map canvas, with the outline as fallback.
- **Linux** primary. cgroup caps via `systemd-run --user`.
- **Nothing in user repos.** All state under `~/.local/share/dovetail/`, config under `~/.config/dovetail/`. Hooks, settings, MCP config and role prompts are passed to Claude Code as launch flags. Never use bare mode.
- **Transparent terminals:** don't paint backgrounds by default.

## Architecture rules (non-negotiable)

- Crates: plain-data API (snapshot, events, commands, `apply()`), core, mock core, TUI, `dovetail` binary. Core and UI talk only by message-passing: `Snapshot` then deltas out, `Command`s in.
- Claude Code sits behind the `SessionDriver` trait; protocol JSON is parsed in `dovetail-driver`. The stream rules (digest §3–§6) are law.
- Keys and `:` commands resolve to one `Action` enum through one keymap table. Storage goes through one DB thread. MCP tool handlers live in the core.

Standards for review: `CODING_STANDARDS.md` (the detail of these rules, code conventions, readability, testing).

## The plan (`docs/implementation-plan.md`)

When asked to plan, write this file with:

1. **Crate layout** and dependency list (each with a one-line reason and features).
2. **Core API** sketches: `Snapshot`, `Event`, `Command`, `apply()`; how it survives a daemon split.
3. **`SessionDriver`** trait sketch, the fake driver, and how events map onto the API.
4. **Storage schema** (tables, FTS, migrations) for v1, with columns v1.1 will need noted.
5. **Spike plan**: for each open spike — what it gates, a concrete procedure, pass/fail criteria, fallback, and who runs it.
6. **Milestones**, in order, each with scope, crates/files touched, a "done when" checklist, tests added, and dependencies on spikes. Mark the **self-hosting point** explicitly, and for each later milestone say which slices it splits into when run inside Dovetail. Suggested shape (reorder with reasons): workspace + API + mock → TUI shell on mock (layout B, modes, keymap, tree, conversation, inbox, picker, quick capture) → storage + project registry → driver + single coordinator session → permissions/inbox live → slices + worktree hooks + cleanup → MCP bridge (`ask_user`, `report_status`) + role prompts → **self-hosting point** → subagents + to-do lists → memory caps + context fill → take-over → maps stage 1 → hardening and daily-driver polish.
7. **Risks and open questions** for the user, including anything contradictory across the docs.

Keep it concrete — real type, module and file names. Then stop and wait.
