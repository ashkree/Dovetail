# AGENTS.md — Dovetail

Instructions for any coding agent working in this repository. Read in this order:

1. this file;
2. `docs/design-v2.md` — the v2 baseline (GUI-first, efforts, canvases, multi-repo); it wins over everything below it. `docs/architecture.md` is the map of crates and data flow;
3. `docs/design-digest.md` — the v1 implementation-relevant design: locked decisions, spike results, protocol facts, open spikes;
4. `docs/design.md` — the full v1 design doc (when present; it wins over the digest);
5. `docs/adr/` — decisions made after the v1 baseline (an ADR wins over the design doc and digest on the decision it names);
6. `docs/gui-layout-and-keys.md`, `mock/gui/` and `docs/screens/gui/` — the GUI's layout, look and keys. The TUI's spec (`docs/ui-session-brief.md`, `docs/screens/ui/`) applies to the frozen TUI only. Screens are reference, the docs are spec;
7. `README.md` — product overview.

## What this is

Dovetail is a keyboard-centric desktop GUI (Tauri 2 + React, core in Rust) for running many Claude Code agents across a **project**: a **coordinator** session the user works with directly, long-lived **slice** sessions (one git worktree each, in one of the project's repos), and the **subagents** they spawn. Work is organised into **efforts** with goals and acceptance criteria; Dovetail tracks tasks, canvases and slice previews, and brings anything that needs the user into one inbox per project. The v1 terminal UI is frozen as a fallback. Sessions run **headless** over Claude Code's NDJSON stream protocol; Dovetail serves its own tools to them over MCP (`dovetail mcp`).

The design phase is finished. **All decisions are locked.** Do not reopen them; if something looks wrong, contradictory or under-specified, stop and ask.

## Scope

The target is v2's first GUI release (`docs/design-v2.md`). Features the design doc parks are out of scope unless a plan in `docs/plan/` says otherwise; leave the seams for them.

Known conflicts, already resolved:
- **Clients:** ADR 0023 supersedes ADR 0020's egui client and `dovetail serve`: the Tauri GUI hosts the core in its own process. The TUI is frozen.
- **Layout:** the GUI follows `docs/gui-layout-and-keys.md`. Decision 15 (Layout B) binds the frozen TUI only.
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

1. **Plan before code.** Write the plan for a build in `docs/plan/<name>.md` (concrete: real type, module and file names; risks and open questions last) and wait for the user's explicit OK.
2. **Spikes before the code that depends on them.** Each open spike in the digest gates specific work. Run it as a small throwaway program under `spikes/<name>/` (not a workspace member), record the result in `docs/spikes/<name>.md`, and report it. Spikes that need the user's machine, account or judgement (terms of use, cgroups on their box) are prepared as runnable scripts and handed to the user.
3. **One wave at a time.** A wave is a set of milestones named in the plan (§6, "Waves"). Agents build a wave's tracks in parallel; at the end of the wave an Opus `code-review` runs from the last review tag, the user runs `retro`, and the wave merges. Milestone "done when" boxes are still ticked one by one. A wave is done when `scripts/check.sh` passes (fmt, clippy, tests and mechanical rule checks), except where the plan relaxes it for that wave (wave 1: `cargo build` only, no new tests; decided 2026-11-05). Then summarise what changed and how to try it.
4. Keep changes scoped to the milestone. No speculative features, no drive-by refactors.
5. User-facing decisions (UX, keys, visible naming) → ask. Implementation details with sensible defaults → decide, note in the summary.
6. Never add a dependency without stating why. Prefer the standard library and small crates.
7. **Never run real Claude Code sessions against the user's real projects** during development. Use a scratch git repo under the target dir or `/tmp`, and start the GUI with `scripts/dev-gui.sh`, which keeps all of Dovetail's state under `target/dovetail-dev/`. Real sessions cost money: keep integration tests behind an opt-in flag (`DOVETAIL_LIVE_TESTS=1`) with tiny prompts.

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

- **Core:** Rust stable, `rusqlite` with `bundled` (SQLite, FTS5, WAL) on a dedicated thread, `tokio` for process/IO in the core only.
- **GUI:** Tauri 2 around React + TypeScript (Vite), React Flow for canvases. The window's TypeScript types are generated from `dovetail-api` (ts-rs) and committed.
- **Low memory:** 16 GB target machine, most of it for Claude Code. Lean dependencies. GUI budget 500 MB (window plus one preview page).
- **Linux** primary. cgroup caps via `systemd-run --user`.
- **Nothing in user repos.** All state under `~/.local/share/dovetail/`, config under `~/.config/dovetail/`. Hooks, settings, MCP config and role prompts are passed to Claude Code as launch flags. Never use bare mode.
- **Frozen TUI:** `ratatui` + `crossterm`, `nucleo`, `pulldown-cmark`; it redraws only when state changed, works in any terminal (no kitty keyboard protocol; kitty graphics only for maps) and paints no backgrounds.

## Architecture rules (non-negotiable)

- Crates: plain-data API (snapshot, events, commands, `apply()`), core, mock core, TUI, `dovetail` binary, and the GUI (`gui/src-tauri`, which hosts the core in-process, and the web frontend in `gui/src`). Core and UI talk only by message-passing: `Snapshot` then deltas out, `Command`s in.
- Claude Code sits behind the `SessionDriver` trait; protocol JSON is parsed in `dovetail-driver`. The stream rules (digest §3–§6) are law.
- Keys and `:` commands resolve to one `Action` enum through one keymap table. Storage goes through one DB thread. MCP tool handlers live in the core.

Standards for review: `CODING_STANDARDS.md` (the detail of these rules, code conventions, readability, testing).

## Plans

`docs/implementation-plan.md` is the v1 plan: its dependency table (§2) is still the one every dependency needs a row in. v2 builds each have their own plan in `docs/plan/`.
