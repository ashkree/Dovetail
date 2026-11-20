# `:handoff` and `:fresh` (task #34)

> Update (task #38, decision #15, which supersedes #10): no fresh start raises a go-back Needs-you item any more. The old session's transcript just stays on disk, and the notice says so. Where this plan describes the item, `Waiter::KeepCoordinator` or `FRESH_COORDINATOR_PERMISSION`, it is out of date.

Two coordinator commands, each reachable as a `:` command and as a chat-bar slash command:

- **`:handoff`** (and `/handoff`): today's `:fresh`, renamed. The coordinator writes its handoff into `journal/<date>.md`, a new coordinator session starts seeded with it, and the old transcript stays on disk (no Needs-you item).
- **`:fresh`** (and `/fresh`): new. A new coordinator session with no handoff and no seed message. The old transcript stays on disk (no Needs-you item).

Decisions (task #34 log): Dovetail intercepts both slash forms, so they never reach Claude Code, and this hides the user's `handoff` skill inside Dovetail (#8). Both act on the coordinator only (#9). The old transcript stays on disk after `:fresh`, with no item (#10, superseded by #15). The fresh session is still the coordinator, with its role prompt and MCP tools (#11). Only an exact `/handoff` or `/fresh` is intercepted; trailing text is refused with a hint (#12).

## Today

- `:fresh` → `Action::FreshStart` (`crates/dovetail-tui/src/command.rs`, `COMMANDS`) → `App::run` pushes `Command::FreshStart` (`app.rs`, the `Action::FreshStart` arm).
- The core handles it in `journal.rs`:
  1. `fresh_start` checks that no turn is running and that the coordinator has a transcript, then calls `ask_for_handoff(Some(time))`.
  2. Once the handoff is written to the journal, `begin_fresh(PendingFresh { kind: Fresh::MidDay, seed })` stops the session.
  3. `coordinator_exited` calls `fresh_coordinator`, which:
     - gives the agent row a new `session_id`;
     - posts a notice;
     - sends the seed with `send_text`, which starts the process;
     - (as built in #34 it raised a go-back item here; removed in #38).
- `/model` in the chat bar is already intercepted: `model_command` in `app.rs` runs before `Command::Prompt` is pushed. Completion lists only Claude Code's commands (`commands_for` → `complete::command_rows`).

## Changes

### API (`dovetail-api`)

- Rename `Command::FreshStart` to `Command::Handoff`. Its behaviour stays as it is.
- Add `Command::FreshSession`: a new coordinator session with no handoff.
- Update `tests/serde_roundtrip.rs`.
- The ts-rs bindings under `gui/src/core/generated` regenerate. If `gui/` has already gone to `gui-shelved` by then, there is nothing to regenerate.

### Core (`dovetail-core`)

- **`journal.rs`:**
  - Rename `fresh_start` to `handoff_start`.
  - Add `fresh_session`. It refuses with the same checks as today: a turn running, the day closing, or a fresh start already pending. If the coordinator has never run, it notes "the coordinator has no session yet, nothing to clear" and stops. Otherwise it calls `begin_fresh(PendingFresh { kind: Fresh::Clean { date, time }, seed: None })`. Nothing is written to the journal; one Activity line records it.
- **`Fresh`** gains `Clean { date, time }`. `PendingFresh.seed` becomes `Option<String>`.
- **`fresh_coordinator(kind, seed: Option<&str>)`:**
  - With a seed, nothing changes.
  - Without one, it calls `ensure_live` after the new `session_id` and does not call `send_text`. The session starts with the role prompt and MCP flags as usual, and the conversation is empty.
  - The notice reads "Fresh coordinator at HH:MM: a new session with no handoff. The previous session's transcript is kept at <path>."
  - No Needs-you item is raised. `restore.rs` still quietly expires such items stored by older builds.
- **`service.rs`:** `Command::Handoff => self.handoff_start()` and `Command::FreshSession => self.fresh_session()`.
- **`dovetail-mock`:** mirror both commands.

### TUI (`dovetail-tui`)

- **`action.rs`:**
  - Rename `Action::FreshStart` to `Action::Handoff`, described as "fresh coordinator seeded with a handoff".
  - Add `Action::FreshSession`, described as "fresh coordinator, no handoff".
  - Both live in the App group.
- **`command.rs` `COMMANDS`:** `("handoff", Action::Handoff)` and `("fresh", Action::FreshSession)`.
- **`activity.rs`:** `Busy::FreshStart` covers both. The text stays "Starting fresh…" and the flag is still `closing && fresh`. A clean start never closes, so `Busy` gains nothing for it; the notice is enough.
- **Coordinator only.** When the focused agent is a slice or side chat, both actions set the note "`:handoff` and `:fresh` act on the coordinator" and send nothing. This holds for the `:` form and the slash form alike.
- **New module `slash.rs`** holds Dovetail's own slash commands in one table, `DOVETAIL_SLASH: &[(name, description, argument_hint)]`:
  - `model`: "Dovetail: pick model and effort", `[name]`;
  - `handoff`: "Dovetail: fresh coordinator seeded with a handoff";
  - `fresh`: "Dovetail: fresh coordinator, no handoff".

  It also provides `fn intercept(text) -> Option<Intercept>`, which replaces `model_command`:
  - `/model [arg]` keeps today's behaviour.
  - An exact `/handoff` or `/fresh` (after trimming) runs `Action::Handoff` or `Action::FreshSession`.
  - `/handoff …` or `/fresh …` with trailing text is refused. The note reads "`/handoff` takes nothing after it; nothing was sent", and the chat text stays in the bar so nothing is lost.
  - Any other text returns `None` and goes to Claude Code as today.
- **`app.rs`:** the chat-key path calls `slash::intercept` where it calls `model_command` today. Attached images are dropped with the same kind of note `/model` uses.
- **`complete.rs`:** `command_rows` takes Dovetail's rows first and then Claude Code's, dropping any of Claude Code's whose name matches a Dovetail row. That hides the user's `handoff` skill and Claude Code's built-in `/model`. Dovetail's rows are marked in the detail column ("Dovetail: …").
- **Status bar:** above 80 % context fill the hint now reads `:handoff` instead of `:fresh` (`docs/ui-session-brief.md` line 75, and wherever the TUI draws it).
- **`:keys` and the docs:** `docs/ui-session-brief.md` (the `:fresh` row in the commands table becomes two rows), `README.md`'s interaction table if it names `:fresh`, and the `AGENTS.md` skill list ("Context running low: `handoff`" becomes "ask the user to run `:handoff`").
- **The coordinator role prompt** names neither command, so it needs no change.

## Tests

The first three exist today and need updating; the rest are new.

- `command.rs`: `parse("handoff")` gives `Action::Handoff` and `parse("fresh")` gives `Action::FreshSession`.
- `tests/journal.rs`: `:handoff` sends `Command::Handoff` and `:fresh` sends `Command::FreshSession`.
- Core `tests/sessions.rs`: the two existing `FreshStart` tests are renamed to `Handoff`.
- New `fresh_session` test: a new `session_id`, no seed sent, the Needs-you item raised, and `y` restores the old id.
- `slash.rs` unit tests, one per `intercept` case: exact match, trailing text, `/model` with an argument, surrounding whitespace, `/handoffs` (no match), and other text.
- `complete.rs`: Dovetail's rows come first, and a Claude Code `handoff` entry is dropped.
- TUI: with a slice focused, both actions send nothing and set the note.
- `dovetail-mock/tests/mock.rs`: both commands.
- `scripts/check.sh` passes.

No live Claude Code runs. The fake driver covers the core tests.

## Risks and open questions

1. **Renaming `Command::FreshStart`** touches the API, core, mock, TUI and tests, plus the generated ts types while `gui/` is still on `main`. **Recommendation: rename**, so `:handoff` maps to `Handoff` and `:fresh` maps to `FreshSession` with no crossed names. Doing it after the GUI is shelved avoids the ts regeneration.
2. **The user's `handoff` skill** can't be reached from Dovetail's chat bar any more (decision #8). The `/` completion shows Dovetail's entry in its place.
3. **Keys.** No key is bound to either command today (`:fresh` is command-only). **Recommendation: keep them command-only.**
4. **`:eod`** keeps its own flow. It is unchanged.
