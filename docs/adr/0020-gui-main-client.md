# The GUI becomes the main client; the TUI becomes a remote kit

Supersedes ADR 0019 (the Companion) on scope, and refines Decision 15 (Layout B), Decision 16 (theme) and the "any terminal" and `ratatui` + `crossterm` hard constraints. The core, the driver, the API, storage, MCP and the role prompts are unchanged.

Why: text in a terminal is harder to read than in a GUI, and density is the main problem. Terminal cells give one font, one size and no line height, margins or measure. The layout and workflow (Layout B, modes, inbox, tree, keys) are fine; the medium is what is rough. ADR 0019 rejected a full GUI because it would throw away keyboard-first use, SSH and the tool Dovetail is built in. This ADR keeps all three: the GUI is keyboard-first through the same keymap, and the TUI stays for SSH.

**Decision:**

- **Two clients, one core.** The GUI is the daily client. The TUI is kept as a **remote kit** for SSH and tmux: read conversations, chat, the inbox (answer permissions and questions), the task tree and the switcher. Everything else (board, maps, reading mode, tangents, research views, activity, worktrees) is GUI only. New features are built in the GUI; the TUI gains one only when it belongs in the remote kit.
- **Same concept.** Layout B as placed by ADR 0006 and ADR 0008, the modes, and every key carry over. The GUI adds GUI-native density: proportional type, a reading measure of about 70 characters, inbox items as cards with their actions, tool summaries as folded rows, and optional mouse use. Mouse is never the only way to do something.
- **One keymap.** Keys resolve to `Action`s through one keymap table shared by both clients (architecture rule). It moves out of `dovetail-tui` into a crate both depend on.
- **Process model.** The client attach from ADR 0019 stays: the GUI talks to the core over the unix socket with `ServerMsg` NDJSON and sends `Command`s back. Which process hosts the core (the GUI, the TUI, or a small headless `dovetail serve`) is settled in the plan; the TUI must be able to attach to a core the GUI started, so SSH reaches a running workspace.
- **Toolkit.** `egui` via `eframe` (ADR 0019's choice) unless the `gui-text` spike fails. Webviews stay rejected on memory.
- **Theme.** One theme model shared by both clients. The GUI adds fonts, sizes and spacing; the TUI keeps foreground colours only and unpainted backgrounds.

**Spike `gui-text` (gates the GUI crate):** render one real, long coordinator transcript (markdown, code blocks, tool summaries) in egui at the mockup's typography (Inter 15 px, line height 1.65, 720 px measure). Pass: text quality judged by the user as close to the HTML mockup, smooth scroll of 2 000 turns, RSS under ~120 MB, no repaint while idle. Fail: compare one alternative toolkit on the same transcript before deciding.

**Open, for the plan:**

- Where the core lives, and whether the TUI attaches to it or can also host it alone.
- The order the GUI buffers land in, and the self-hosting point for the GUI (the point after which Dovetail is built from the GUI).
- What the TUI drops and when: frozen code stays until the GUI covers it, then is removed.
- Fate of the kitty map canvas (frozen by ADR 0019): kept, or removed once maps live in the GUI.

Mockup: the "Dovetail GUI concept" canvas (Layout B, GUI-native; dark/light and comfortable/compact tweaks).

Approved by the user in the session of 2026-11-15 (GUI as main client, TUI as remote kit, egui gated by `gui-text`).
