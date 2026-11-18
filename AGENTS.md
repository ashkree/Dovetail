# AGENTS.md — Dovetail

Instructions for any coding agent working in this repository. Read in this order:

1. this file;
2. `docs/adr/0030-tui-main-client.md` — the TUI is the main client; the order of work. It wins over everything below on the client;
3. `docs/design-v2.md` — the v2 baseline (efforts, multi-repo projects, canvases); it wins over the v1 docs. `docs/architecture.md` is the map of crates and data flow;
4. `docs/design-digest.md` — the v1 implementation-relevant design: locked decisions, spike results, protocol facts, open spikes;
5. `docs/design.md` — the full v1 design doc (when present; it wins over the digest);
6. `docs/adr/` — decisions made after the v1 baseline (an ADR wins over the design doc and digest on the decision it names);
7. `docs/ui-session-brief.md` and `docs/screens/ui/` — the TUI's layout, look and keys. Screens are reference, the docs are spec;
8. `README.md` — product overview.

## What this is

Dovetail is a keyboard-centric terminal UI (Rust, `ratatui`) for running many Claude Code agents across a **project**: a **coordinator** session the user works with directly, long-lived **slice** sessions (one git worktree each, in one of the project's repos), and the **subagents** they spawn. Work is organised into **efforts** with goals and acceptance criteria; Dovetail tracks tasks and brings anything that needs the user into one inbox per project. The core will also serve one local web page: a reader for long reports and diagrams, then canvases. Sessions run **headless** over Claude Code's NDJSON stream protocol; Dovetail serves its own tools to them over MCP (`dovetail mcp`). `dvtl` is a short name for the `dovetail` binary.

The design phase is finished. **All decisions are locked.** Do not reopen them; if something looks wrong, contradictory or under-specified, stop and ask.

## Scope

Work follows ADR 0030 §7, one step at a time: readability spike and TUI design session → v2 views in the TUI → the page (reader, then canvas) → the daemon (`dovetail serve`). No screen code until the design session is done. Features the design docs park are out of scope unless a plan in `docs/plan/` says otherwise; leave the seams for them.

Known conflicts, already resolved:
- **Clients:** ADR 0030 supersedes ADR 0020 and ADR 0023 on the client. The TUI hosts the core in-process until the daemon lands. The Tauri GUI is shelved: `gui/` moves to the `gui-shelved` branch; build nothing new in it.
- **Layout:** Decision 15 (Layout B) binds the TUI as built; the design session settles the new main layout.
- **Edition:** the workspace uses edition 2024 (already in `Cargo.toml`).

## Dogfooding: Dovetail builds Dovetail

Dovetail is built inside the TUI. The repo's own role prompts and recipe are used for real, so they must stay accurate. Sources of truth:
- `config/roles/coordinator.md`, `config/roles/slice.md` — role prompts (keep each under ~20 lines, Decision 8c). Dovetail reads the installed copies in `~/.config/dovetail/roles/`.
- `config/projects/dovetail.toml` — the worktree recipe for this repo; the core reads it.
- `scripts/bootstrap.sh` — creates Dovetail's dirs, installs the prompts and recipe without overwriting user edits, and checks prerequisites (Claude Code ≥ 2.1.224, systemd-run caps, messaging blockers). Keep it in step with any path or format change.

Self-hosting never weakens the rules: slices working on Dovetail still get their own worktree, and nothing is written into the repo at runtime beyond branches.

## How to work

1. **Plan before code.** Write the plan for a build in `docs/plan/<name>.md` (concrete: real type, module and file names; risks and open questions last) and wait for the user's explicit OK.
2. **Spikes before the code that depends on them.** Run each as a small throwaway program under `spikes/<name>/` (not a workspace member), record the result in `docs/spikes/<name>.md`, and report it. Spikes that need the user's machine, account or judgement are prepared as runnable scripts and handed to the user.
3. **Done** means `scripts/check.sh` passes (fmt, clippy, tests and mechanical rule checks). Then summarise what changed and how to try it.
4. Keep changes scoped to the task. No speculative features, no drive-by refactors.
5. User-facing decisions (UX, keys, visible naming) → ask. Implementation details with sensible defaults → decide, note in the summary.
6. Never add a dependency without stating why. Prefer the standard library and small crates.
7. **Never run real Claude Code sessions against the user's real projects** during development. Use a scratch git repo under the target dir or `/tmp`, and keep Dovetail's state under `target/dovetail-dev/` by setting `DOVETAIL_DATA_DIR`, `DOVETAIL_CONFIG_DIR`, `DOVETAIL_STATE_DIR` and `DOVETAIL_RUNTIME_DIR` (see `scripts/dev-gui.sh` until a TUI dev script replaces it). Real sessions cost money: keep integration tests behind an opt-in flag (`DOVETAIL_LIVE_TESTS=1`) with tiny prompts.

## Conventions for skills

- **Standards:** `CODING_STANDARDS.md`, plus the hard constraints and architecture rules here. **Spec:** the task's plan in `docs/plan/`, ADR 0030 and `docs/design-v2.md`.
- **Decisions:** the decision log in `docs/design.md` is the frozen v1 baseline, never edited. New decisions that meet the `domain-modeling` ADR bar go in `docs/adr/NNNN-slug.md`, linking the spike or discussion behind them. An ADR may refine or supersede a locked decision only with the user's approval in the session, and must name the decision it changes.
- **Terms:** `GLOSSARY.md` at the repo root.
- **Spikes:** code in `spikes/<name>/`, results in `docs/spikes/`, never `prototype/` branches.
- **Commits:** the agent commits, merges to `main` and pushes on its own judgement (decided 2026-11-05).

Which skill when:

- **Reviews:** `code-review`, then `retro`.
- **Failures:** `diagnosing-bugs`.
- **Research:** `research`.
- **Context running low:** `handoff`.
- **Slice PRs:** the convention in `config/roles/slice.md`.

`retro` and `handoff` are user-invoked: ask the user to run them.

## Hard constraints

- **Core:** Rust stable, `rusqlite` with `bundled` (SQLite, FTS5, WAL) on a dedicated thread, `tokio` for process/IO in the core only.
- **TUI:** `ratatui` + `crossterm`, `nucleo`, `pulldown-cmark`. It redraws only when state changed. The target terminal is kitty: the text-sizing protocol and kitty graphics (maps) are used there and fall back quietly elsewhere and under tmux; readability never depends on them. No kitty keyboard protocol. Transparent by default; solid backgrounds on content panes are a toggle (ADR 0030 §5).
- **Page:** one small HTTP server in the core, bound to `127.0.0.1` on a free port, serving a single-page app (React + TypeScript, React Flow for canvases). Its types are generated from `dovetail-api` (ts-rs) and committed. Keyboard-first; the mouse is never the only way.
- **Low memory:** 16 GB target machine, most of it for Claude Code. Lean dependencies. About 100 MB RSS for the TUI with its core; the browser tab is not budgeted.
- **Linux** primary. cgroup caps via `systemd-run --user`.
- **Nothing in user repos.** All state under `~/.local/share/dovetail/`, config under `~/.config/dovetail/`. Hooks, settings, MCP config and role prompts are passed to Claude Code as launch flags. Never use bare mode.
- **Names:** the binary, the launch flags sessions use (`dovetail mcp`, `dovetail hook`) and the state and config paths are `dovetail`; `dvtl` is only a symlink.

## Architecture rules (non-negotiable)

- Crates: plain-data API (snapshot, events, commands, `apply()`), core, mock core, driver, maps, TUI, and the `dovetail` binary; the page's frontend joins them when it is built. Core and clients talk only by message-passing: `Snapshot` then deltas out, `Command`s in.
- Claude Code sits behind the `SessionDriver` trait; protocol JSON is parsed in `dovetail-driver`. The stream rules (digest §3–§6) are law.
- Keys and `:` commands resolve to one `Action` enum through one keymap table. Storage goes through one DB thread. MCP tool handlers live in the core.

Standards for review: `CODING_STANDARDS.md` (the detail of these rules, code conventions, readability, testing).

## Plans

`docs/implementation-plan.md` is the v1 plan: its dependency table (§2) is still the one every dependency needs a row in. v2 builds each have their own plan in `docs/plan/`.
