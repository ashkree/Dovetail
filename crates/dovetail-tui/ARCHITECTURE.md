Dovetail's terminal UI: buffers, regions, keymap, overlays, theme and rendering.

`dovetail-tui` depends only on `dovetail-api` (never `dovetail-core`, `dovetail-driver`, or `dovetail-tools`). It holds its own copy of the [`dovetail_api::Snapshot`], mutates it only through [`dovetail_api::apply`], and asks the core for changes by queuing [`dovetail_api::Command`]s. It never spawns processes or writes files; lending the terminal (`$EDITOR`, native take-over) is coordinated with the core through `AppCommand` / `AppEvent` handshakes.

# Layout

```text
src/
  lib.rs            crate root; re-exports App, Clock, run, Mode, Theme, capture
  event_loop.rs     terminal setup (raw mode, alt screen, DISAMBIGUATE_ESCAPE_CODES),
                    event/signal/wake loop, kitty graphics sync, terminal handoff
  fmt.rs            time, byte/token/duration formatting, truncation, word wrap,
                    wrapped_lines and line_plain_text
  theme.rs          colour palette, semantic styles and transparent/solid background toggle
  app/              plain-state application controller (no terminal or threads)
    mod.rs            App, Pending (in-flight handshakes), event/key/mouse dispatch, settle()
    draw.rs           frame layout, screens bar, status bar, prompt row, overlay dispatch
    focus.rs          active agent/task resolution, command dispatch, permission mode, take-over
    overlay.rs        Overlay enum, chat-mark restore, picker/menu/capture/confirm handling
    panel.rs          side panel open/close and side-chat switching
    projects.rs       project switcher, :project new, profile picker
    reading.rs        reading mode lifecycle, library loading, citation following, Ask
    search.rs         in-buffer / search with smart-case matching
    tangents.rs       tangent creation, reading, filing, attaching
  keymap/           keys, modes, commands and help
    mod.rs            re-exports Action, Mode and keymap table items
    table.rs          Key, KeyError, KeymapError, Scope, Binding, Keymap, Resolver
    action.rs         one Action enum for every keybinding and : command, with groups/descriptions
    mode.rs           Mode (Normal, Insert, Command, Map)
    command.rs        : command table, parse(), and typed argument parsers (RoundArg, etc.)
    help.rs           which-key popup and :keys picker items/previews
  input/            text editing and chat bar
    mod.rs            re-exports TextInput, InputOutcome, ChatInput, ChatKeyOutcome
    text.rs           single-line and multiline TextInput with visual-line wrapping
    chat.rs           ChatInput: multiline text, pasted image chips, @ completion, / handoff
    complete.rs       @ path completion popup ranked by nucleo
    slash.rs          / command interception for Dovetail-owned slash commands
  overlay/          floating modal boxes
    mod.rs            centered_rect() and overlay submodules
    picker.rs         generic fuzzy Picker<K>, PickerItem<K>, PickerOutcome<K>
    switcher.rs       SwitchKey (Agent, Task, Inbox, Command, Project), item builders & previews
    mcp_menu.rs       MCP server menu over Picker<String> (status, actions, OAuth paste)
    usage_menu.rs     account usage limits menu over Picker<usize>
    model_select.rs   model/effort picker over Picker<ModelKey>
    capture.rs        quick-capture task parser and overlay (Capture::render)
    confirm.rs        2-button confirm modal for switching the coordinator's driver
  layout/           screen geometry
    mod.rs            region and tabs submodules
    region.rs         RegionId, BufferKind, Preset (LayoutB, Reading), Regions, screen splits
    tabs.rs           top screens bar tab definitions and width-adaptive rendering
  chrome/           status-bar indicators
    mod.rs            activity and usage submodules
    activity.rs       spinner frames, woodworking words, tool labels, Activity timers
    usage.rs          usage-limit status bar variants (graceful width degradation)
  buffers/          views shown inside regions (Buffer trait, ViewCx, Effect, Buffers)
    mod.rs            Buffer trait, Effect, Buffers container, pane/title/footer/body_height helpers
    list.rs           shared ListCursor<K> for ID-keyed list selection and viewport scroll
    tasks/            tree, board, task_detail, task_research, worktrees
    chat/             conversation, side_panel, feed (ActivityBuffer, MessagesBuffer)
    inbox/            list (InboxBuffer), inbox_detail
    reading/          library, reader, ask, tangents
    maps/             map_shelf, map_outline, map_canvas
  render/           pure line/span builders (no terminal I/O)
    mod.rs            submodules
    markdown.rs       pulldown-cmark to styled ratatui Lines
    html.rs           lightweight HTML-to-markdown/text cleanup
    citations.rs      [F n] / [S n] citation detection and styling
    reading_column.rs centered reading column geometry
    transcript.rs     conversation entry rendering, folding and line cache
    working.rs        working-line and busy-command spinner spans
    wrap.rs           styled-span word wrapping
  canvas/           kitty graphics protocol support (graceful fallback elsewhere/tmux)
    mod.rs            MapPainter trait, Picture, Placed, PxRect, PaintJob, Painted
    detect.rs         terminal/cell-pixel capability probing
    kitty.rs          chunked base64 (t=d / t=f) kitty graphics commands and Screen diffing
    ring.rs           RGBA selection ring generator for map nodes
    view.rs           pan/zoom/fit viewport math
```

# Data flow

```text
  terminal keys / mouse / resize / signals
        │
        ▼
  event_loop::run ──► App (on_key / on_mouse / on_signal / on_tick)
        ▲                  │
        │                  ├──► Keymap / Resolver ──► Action ──► Buffers::get_mut(kind)
        │                  │                                         │
        │                  │                                    Vec<Effect>
        │                  │                                         │
        │                  ├──► outbox: Vec<Command> ──► CoreLink ───┘
        │                  │
        │                  └──► draw(frame): Regions ──► Buffers::render + Overlay::render
        │
  ServerMsg (Snapshot / Event) ──► App::on_server ──► dovetail_api::apply() ──► settle()
```

- [`App`] is pure state with no terminal or background threads, so integration tests drive it directly with `KeyEvent`s and `ServerMsg`s and assert on rendered buffers and queued `Command`s.
- [`event_loop::run`] only redraws when `App` marks itself dirty, and schedules wakes only for active timers (key sequence timeout, which-key delay, Ctrl-C exit window, spinner frames, usage reset boundaries).
- `App::settle()` maintains invariants after every state change (exiting focus mode when focus moves to a side region, exiting MAP mode if the map closes, keeping `ConversationBuffer` on a live agent, and clearing `FinishedTurn` inbox items when a conversation is focused). `App::draw()` only renders.

# Keymap and actions

Every keybinding and every `:` command resolves to one [`keymap::Action`] enum through a single table in `keymap/table.rs` (`BINDINGS`) and `keymap/command.rs` (`COMMANDS`). Buffer-local bindings (`Scope::Buffer(kind)`) override `Scope::Global` bindings on the same key sequence. Half-typed sequences wait up to `SEQUENCE_TIMEOUT_MS` and show the which-key popup after `WHICH_KEY_DELAY_MS`. Argument parsing for `:` commands (`:round`, `:upgrade`, `:spawn`, `:research kind`, `:project new`) lives in `keymap/command.rs` so `app/focus.rs` dispatches typed values.

# Generic `Picker<K>` and overlays

[`overlay::picker::Picker<K>`] is parameterized by its row key type `K` so each overlay owns its own strongly-typed selection without a monolithic key enum:

- Switcher / finder / relate / project / tangent pickers use `Picker<SwitchKey>` (`Agent`, `Task`, `Inbox`, `Command`, `Project`).
- `:keys` uses `Picker<Action>`.
- `:project add` profile selection uses `Picker<Option<String>>`.
- `McpMenu` uses `Picker<String>`, `UsageMenu` uses `Picker<usize>`, and `ModelPicker` uses `Picker<ModelKey>`.

All modal overlays share `overlay::centered_rect` for positioning. `App` records a `ChatMark` when an overlay opens from an active chat bar and restores Insert mode on close only if focus, chat site and agent still match.

# Rendering and kitty graphics

`render/` turns markdown, transcripts, citations and working indicators into `ratatui::text::Line`s without touching the terminal. `canvas/` handles optional kitty graphics for structure/swimlane maps and reading-mode figures: `canvas/kitty.rs` encodes payloads with the workspace `base64` crate (and `OsStrExt::as_bytes()` for `t=f` file paths), chunks them at 4096 bytes with `q=2`, and diffs against the last drawn `Screen` so unchanged images are never resent. On terminals without kitty graphics (or under tmux), maps fall back to the text outline and figures are omitted.
