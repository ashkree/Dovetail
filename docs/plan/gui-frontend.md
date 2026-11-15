# GUI frontend: implementation plan

**Status:** proposed 2026-11-15, waiting for the user's OK. Written for a build session on a smaller model: follow it in order, and stop and ask where it says to.

**What this builds:** the Dovetail GUI's web frontend (React + TypeScript), running in a browser from a dev server, on **fake data**. It is the real product code for the frontend, not another mock. The Tauri shell and the Rust core are wired in later, through one seam (`CoreLink`, §3).

**Sources, in order of authority:**

1. `docs/gui-layout-and-keys.md` — layout, look and keys. It wins on those.
2. `docs/design-v2.md` (PR #73; read it from that branch if it is not merged yet) — nouns and behaviour. It wins on those.
3. `docs/screens/gui/01`–`11*.png` — what each screen must look like at 1600 × 950.
4. `mock/gui/` — the clickable reference. Run `node mock/gui/serve.mjs` and keep it open while building. Port its **behaviour, data and CSS values**; do not port its structure (it is HTML strings in one file).
5. `AGENTS.md` and `CODING_STANDARDS.md` — repo rules. Their TUI-specific constraints (ratatui, "any terminal") do not apply to `gui/`.

## 1. Scope

In scope: Home, the workspace (chat, main view, sidebar), the inbox and inbox item, the efforts tree, the "where was I" pop-up, the question-tree canvas with the incoming strip, the viewer with the variant panel, the reading view, modes and keys, the Space leader with which-key, the `?` keyboard overlay, the picker and the `:` command line.

Out of scope, do not build: Tauri, any Rust change, a backend or network call, real Claude sessions, the board, maps, daily review, tangents, side chats, take-over, quick capture, an agent's own conversation (opening one shows a note, as in the mock), the options-compare and free-board canvas frameworks (leave the framework field in the model; build only the question tree).

## 2. Stack and dependencies

Everything lives in a new top-level folder `gui/`. It is an npm project and is not part of the Cargo workspace. Node ≥ 23 and npm are on the machine.

| Package | Kind | Why |
| --- | --- | --- |
| `react`, `react-dom` | runtime | The UI library (design v2) |
| `@xyflow/react` | runtime | React Flow: the canvas (design v2) |
| `react-markdown`, `remark-gfm` | runtime | Chat and reading-view markdown as React elements, so no raw HTML is injected; `remark-gfm` for tables |
| `@fontsource/atkinson-hyperlegible` | runtime | The font, bundled; the app must not load fonts from the network |
| `vite`, `@vitejs/plugin-react` | dev | Dev server and build |
| `typescript`, `@types/react`, `@types/react-dom` | dev | Types, `strict: true` |
| `vitest` | dev | Unit tests for the two pure modules (§3 `apply`, §4 key resolver) |

No other dependency. No state library, CSS framework, router, icon set or component kit. If something seems to need one, **stop and ask**.

## 3. Architecture

The frontend mirrors the Rust core's contract (`crates/dovetail-api`): the client holds a `Snapshot`, receives `Event`s with a sequence number, folds them in with a pure `apply()`, and sends `Command`s. Nothing else changes shared state. This is the seam that lets the real core replace the fake one without touching components.

```
components ──Command──► CoreLink ──► MockCore (now)   /  TauriCore (later, not in this plan)
     ▲                     │
     └── Snapshot ◄─ apply(snapshot, seq, event) ◄─ ServerMsg
```

```
gui/
├── index.html, package.json, tsconfig.json, vite.config.ts
├── public/sample/            # a small static page the viewer shows (§6 F7)
└── src/
    ├── main.tsx, App.tsx
    ├── core/
    │   ├── model.ts          # Snapshot and its parts (below)
    │   ├── protocol.ts       # ServerMsg, Event, Command (discriminated unions)
    │   ├── apply.ts          # pure reducer; apply.test.ts
    │   ├── link.ts           # interface CoreLink { subscribe(fn): () => void; send(cmd): void }
    │   ├── mock.ts           # MockCore implements CoreLink, with scripted replies
    │   └── fixtures.ts       # the fake data, ported from mock/gui/data.ts
    ├── state/
    │   ├── store.ts          # holds Snapshot; useSyncExternalStore; one CoreLink
    │   └── ui.ts             # UI-only state: screen, zone in focus, mode, pending keys,
    │                         #   selections, open view, modal, sidebar width and collapse
    ├── keys/
    │   ├── keymap.ts         # THE one table, ported from mock/gui/keymap.ts
    │   ├── actions.ts        # type Action (union of action names) and the dispatcher
    │   ├── resolve.ts        # key event → token → sequence → Action; resolve.test.ts
    │   └── useKeys.ts        # one window keydown listener
    ├── styles/tokens.css     # :root from mock/gui/style.css, unchanged
    ├── styles/*.css          # one plain CSS file per area, ported from mock/gui/style.css
    └── components/
        ├── shell/            # TopBar, BottomBar, Workspace (the three-zone grid), Toast
        ├── home/             # Home
        ├── sidebar/          # Sidebar, Inbox, EffortsTree, Rail
        ├── chat/             # Chat, Message, ReportLine, ChatBar, EffortTag, Markdown
        ├── main/             # InboxItemView, ReadingView, WhereWasI
        ├── canvas/           # Canvas, CardNode, IncomingStrip
        ├── viewer/           # Viewer, VariantPanel
        └── overlays/         # WhichKey, KeyboardOverlay, Picker, CommandLine
```

**Model (`core/model.ts`).** Start from the types in `mock/gui/data.ts` and reshape them to design v2's nouns. Use ids, not array positions or names, for every reference. Use `snake_case` field names, because that is what the Rust API serialises; the exact shapes are reconciled when the real core is wired.

- `Snapshot { seq, projects, open_project, efforts, tasks, agents, inbox, chat, canvases, cards, card_links, incoming, variants, reports, status }`
- `Effort { id, name (lower kebab case), kind, question_or_goal, running, where_was_i { last, waits, next }, criteria: Criterion[] }`, `Criterion { id, text, state: 'open' | 'claimed' | 'met' }`
- `Task { id, effort_id, title, status: 'todo' | 'in_progress' | 'review' | 'done' }`, `Agent { id, task_id, parent_id?, kind, name, state, branch? }`
- `ChatEntry { id, effort_id?, who: 'you' | 'coordinator' | agent id, report: boolean, at, text }`
- `Card { id, canvas_id, kind, x, y, title, body, ref?, long }`, `CardLink { from, to }`
- `Status` holds everything the bottom bar shows: effort limit, profile, context fill, usage, model, permission mode.

**Commands and events (`core/protocol.ts`).** Only what the UI needs now: `open_project`, `send_message { text, effort_id }`, `answer_inbox { id, answer, reason? }`, `place_card`, `drop_incoming`, `move_card`, `delete_card`, `add_card`, `add_task`, `rename_effort`, `new_effort { name, kind }`, `keep_variants { choice }`. Each has a matching event (`chat_entry_added`, `inbox_resolved`, `card_moved`, …). `MockCore` answers every command by emitting events with the next `seq`; for `send_message` it also emits a short coordinator reply after 600 ms. `apply` rejects an event whose `seq` is not `snapshot.seq + 1` (return a gap result; the store then asks for a fresh snapshot), as `crates/dovetail-api/src/apply.rs` does.

**Rules that keep it honest:**

- Components never mutate the snapshot and never import `mock.ts` or `fixtures.ts`; they call `send(command)`.
- UI state (focus, mode, selection, open view) lives in `state/ui.ts`, never in the snapshot.
- No timers or polling except `MockCore`'s scripted replies. No `dangerouslySetInnerHTML`. No network requests at runtime.
- Effort names: typed names are converted to lower kebab case before the command is sent.

## 4. Keys

- `keys/keymap.ts` is a port of `mock/gui/keymap.ts`: scopes, bindings `[keys, action, label, category]`, and the Space-leader tree. It is the only place a key is named. **The keymap is a proposal the user has not reviewed**, so keep it pure data: changing a key must be a one-line edit in this file and nowhere else.
- `resolve.ts` turns a `KeyboardEvent` into a token (`j`, `C-h`, `<Esc>`, `<Space>`), keeps the pending sequence (`g` then `g`, `]` then `e`, the leader path), and returns an `Action` or "pending" or "none". Scope bindings win over global ones. The scope comes from the zone in focus and the open view; Insert mode is its own scope.
- Every action is a member of the `Action` union and is run by one dispatcher in `actions.ts`. Mouse handlers call the same dispatcher. Nothing is reachable by mouse only.
- The which-key pop-up and the `?` overlay are drawn from the same table, so they cannot drift from what the keys do.
- React Flow's own keyboard handling is turned off; the canvas keys come from the table.
- Tests: `resolve.test.ts` covers a single key, a two-key sequence, a leader path, scope-over-global, and Insert mode passing text through.

## 5. Look

Copy `:root` from `mock/gui/style.css` into `styles/tokens.css` without changing a value, and port the rest of the mock's CSS area by area. The "Look" table in `docs/gui-layout-and-keys.md` is the spec: Mist palette on dark greys, framed panels with 4 px corners 8 px apart, Atkinson Hyperlegible 14 px at line height 1.5, all statuses in the bottom bar. Do not restyle, add colours, or bring back anything listed there as rejected. Import the font from `@fontsource/atkinson-hyperlegible` in `main.tsx`.

## 6. Milestones

Do them in order. After each: `npm run check` passes (§7), compare the screen with its screenshot in `docs/screens/gui/` and with the running mock, then commit with a message naming the milestone.

**F1. Scaffold and shell**
- `gui/` with Vite, React, strict TypeScript, tokens, the font. `npm run dev` serves on `http://127.0.0.1:5173` (`strictPort`, host `127.0.0.1`).
- `TopBar`, `BottomBar` and the workspace grid (chat 440 px · main · sidebar 330 px; without a main view the chat takes its space) with static content.
- Add `gui/node_modules` and `gui/dist` to `.gitignore`.
- Done when: the empty framed shell matches `02-workspace-chat.png` in structure.

**F2. Model, MockCore, store**
- `core/*` and `state/store.ts`; fixtures ported from `mock/gui/data.ts` (project Shop, repos shop-api and shop-web, five efforts, the inbox items, chat, cards, variants, the report).
- Done when: `apply.test.ts` passes (an event applies; a gap is rejected; a stale event is ignored) and the shell shows real fixture values in both bars.

**F3. Modes and keys**
- `keys/*`, `state/ui.ts`, Normal and Insert modes, zone movement (`h l`, `Ctrl-h/j/k/l`), the leader with `WhichKey`, the `?` `KeyboardOverlay` (layers for the scope in focus and for the leader), `Esc`.
- Done when: `resolve.test.ts` passes; the mode and pending keys show in the bottom bar; screens `09`, `10`, `11` match.

**F4. Home**
- Recent tiles and the project list; `j k gg G Enter`; opening a project shows the workspace; `Space p h` returns.
- State in the URL hash as the mock does (`#screen="ws"&main="canvas"&effort="pay"`), so any screen can be opened directly for comparison.
- Done when: `01-home.png` matches.

**F5. Sidebar and inbox item**
- Inbox (grows with its items, up to 40 % of the height) over the efforts tree (efforts → tasks → agents, fold with `o`); collapse to the rail (`Space z`); drag the left edge to widen.
- `Enter` on an inbox row opens `InboxItemView` in the main view; `y A d D r x` send `answer_inbox`; an approved criterion shows as met in the tree and the pop-up.
- Done when: `07` and `08` match, and answering an item removes it and updates the bottom bar's count.

**F6. Chat and "where was I"**
- Messages with speaker, effort tag, rule and time; you and coordinator colours; slice reports as one collapsed line (`zo`, `zc`); the effort filter (`f`); `]] [[ gg G`.
- `ChatBar`: Insert mode, `Enter` sends with the shown effort tag, `Alt-Enter` new line, `Ctrl-t` changes the tag. The mock reply arrives through events.
- `WhereWasI` pop-up on switching effort (`]e [e`, the tree, `Space e e`); `w` and `Space e w` bring it back.
- Done when: `02` and `03` match; a sent message appears tagged, and the filter shows only that effort's thread.

**F7. Reading view and viewer**
- `ReadingView`: the fixture report as markdown, `]] [[` by heading, `r` puts a reference in the chat bar.
- `Viewer`: an `<iframe>` of `public/sample/index.html`, a small static page whose theme, header, footer and hero text change when the parent posts the chosen variants (`postMessage`). `VariantPanel` with the three levels, `j k Tab 1 2 3`, detach and drag, close, `y` keep (sends `keep_variants`), `u` back to kept.
- Done when: `05` and `06` match, and changing a variant changes the sample page.

**F8. Canvas**
- React Flow with a custom `CardNode` per card kind (edge colours from the tokens) and the fixture links; pan and zoom by mouse; `+ - 0`.
- Keyboard: `h j k l` select the nearest card in that direction (port `nearestCard` from `mock/gui/app.ts`), `H J K L` move the card, `Tab`/`Shift-Tab` follow links, `Enter` opens a long card in the reading view, `r` and the card's "↗ chat" button insert its reference into the chat bar, `n`, `dd`.
- `IncomingStrip` under the canvas: `gi`, `h l`, `a` place, `x` drop.
- The agent never moves the user's cards: only `move_card` from the user changes a position.
- Done when: `04` matches; a card can be placed, moved and referenced by keyboard alone.

**F9. Picker, command line, finish**
- `Picker` (`Space Space`, effort and project switchers), `CommandLine` (`:`) with the mock's commands, `Toast`.
- `gui/README.md`: how to run, the folder map, how the real core will plug in (`CoreLink`).
- Walk every screen against `docs/screens/gui/` and every table in "Keys" against the running app; list any gap in the PR.
- Done when: `npm run check` passes, `scripts/check.sh` still passes, and the PR is open.

## 7. Checks and working rules

- `gui/package.json` scripts: `dev`, `build`, `test` (`vitest run`), `typecheck` (`tsc --noEmit`), `check` (typecheck, test, build).
- Do not edit anything outside `gui/`, `.gitignore` and this file's milestone ticks. Do not edit `mock/gui/`, the Rust crates or the design docs.
- Layout, look, keys and visible names are the user's decisions and are settled in `docs/gui-layout-and-keys.md`. Where it is silent or seems wrong, **stop and ask**; do not invent.
- Serve the app with `npm run dev` and show it to the user in the browser as soon as F1 runs; keep it running and tell them the URL. After each milestone, say in two lines what is new and how to try it.
- Commit after each milestone on the session's branch, push, and open one PR at the end. Do not merge it.

## 8. Later, not in this plan

- `TauriCore implements CoreLink`, the Tauri 2 shell hosting `dovetail-core` in the GUI process, and TypeScript types generated from `dovetail-api`.
- The v2 core work: efforts, criteria, effort tags, canvases and cards in the API and database.
- The memory spike for the 500 MB budget.
- The keymap review round with the user.
