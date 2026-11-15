# The Companion: a GUI window beside the TUI

Refines Decision 23 (visual maps), Decision 4b (process ownership) and the "any terminal" hard constraint. The TUI stays Dovetail's main and complete interface. Visual work that a terminal does badly gets a second client, the **Companion**: a native GUI window that attaches to the running app. Maps are its first buffer.

Why: maps, and later other visual views, want real pointer interaction, smooth pan and zoom, hit-testing on drawn objects and layouts Dovetail controls. The kitty canvas (M15 f) works, but it pans a cached picture, draws the selection as a second image and needs merman's undocumented node positions. A full GUI was the other option. It was rejected because it throws away the keyboard-first TUI, SSH and tmux use, and the tool Dovetail is being built in. A webview (Tauri) was rejected on memory: 150–300 MB per window against the 16 GB budget.

**Decision:**

- **Toolkit:** `egui` via `eframe`, in a new crate `dovetail-companion`. Pure Rust, small, and it repaints only on input or on a new event, which keeps the "no idle ticks" rule. `ratatui` + `crossterm` stay the TUI's toolkit. `egui` is allowed only in the Companion crate.
- **Separate process:** `dovetail companion` attaches to the running app's unix socket (`$XDG_RUNTIME_DIR/dovetail/<project-id>.sock`) and receives the same `ServerMsg` stream (`Hello`, `Snapshot`, `Event { seq }`) as NDJSON. It sends `Command`s back and recovers from a `Gap` with `Resync`, exactly as the API was designed for the daemon split. The TUI process still hosts the core. This brings the **client attach** forward from "Later". The full daemon (core without a TUI, remote attach) stays later. Closing or crashing the Companion never touches sessions.
- **Optional, never required:** everything stays reachable from the TUI in any terminal. The Companion adds views. It never holds the only way to do something, and nothing in the TUI waits on it.
- **Maps:** the kitty canvas and MAP mode are **frozen** as built in M15 (f), together with the outline and its fallback role. New map features (stage 2 and later, mouse editing, comments) are built in the Companion only. The structured map model, `maps_draw` and `MapRenderer` are shared and unchanged. The Companion may draw maps itself from the model instead of going through a PNG.
- **Keys:** the Companion has its own key and mouse handling, but its keys resolve to `Action`s through the same keymap table as the TUI's (architecture rule). Splitting the keymap out of `dovetail-tui` is part of the first Companion milestone.
- **Layout unchanged:** Decision 15, ADR 0006, ADR 0008 and ADR 0018 still govern the TUI. The Companion is a window of its own, not a region.

**Open, for the plan:**

- How the TUI points the Companion at something (e.g. "open this map in the Companion"). It may be a core-relayed hint, since `Action`s never cross the boundary. The key for it is the user's call.
- Which buffers follow maps (the board, reviews and image-heavy reading are candidates), and in what order.
- Which project the Companion attaches to when several Dovetail apps run: the one for the current folder, or a picker.
- Where the milestone sits in the plan, and whether the M15 "done when" item for kitty is still ticked as is.

Approved by the user in the session of 2026-11-11 (toolkit, process model, frozen kitty canvas and the name "Companion").

Note: superseded on scope by ADR 0020. The GUI is now the main client and the TUI a remote kit; the socket attach and toolkit choice carry over.
