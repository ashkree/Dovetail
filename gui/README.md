# Dovetail GUI frontend

The web frontend of the Dovetail GUI (React, strict TypeScript, Vite), running on **fake data** in a browser. It is the product code for the frontend; the Tauri shell and the Rust core plug in later through one seam, `CoreLink`. Plan: `docs/plan/gui-frontend.md`. Layout, look and keys: `docs/gui-layout-and-keys.md`. Decisions taken while building it unattended: [`DECISIONS.md`](DECISIONS.md).

## Run it

```bash
cd gui
npm install
npm run dev        # http://127.0.0.1:5174
```

`npm run check` runs the type check, the unit tests and a production build. Nothing is fetched from the network at runtime (the font is bundled).

Any screen opens straight from the URL hash, as in the mock: `#screen=ws&main=canvas&effort=2`, `#screen=ws&main=item&mainItem=c5&zone=main`, `#screen=ws&main=viewer&effort=4&choice=[0,0,1,0,2]`, `#screen=ws&leader=[]`. Values are JSON or plain strings; any field of `state/ui.ts` works, plus `project` (default: the first project). Effort ids are the core's numbers (in the mock: 2 payment-providers, 3 login-bug, 4 dark-mode, 5 search-index, 6 coupon-ab); inbox rows are `i<id>`, or `c<criterion>` for a claim to review.

## Try it

Everything works by keyboard; the mouse is optional. `?` draws the keyboard for the view in focus (`[` `]` change view, `Tab` changes layer). Home: `j k Enter`. In the workspace: `h l` move between chat, main view and sidebar, `Space` opens the leader (`Space v c` canvas, `Space v w` viewer, `Space v r` reading, `Space e e` switch effort, `Space n` inbox), `i` types in the chat bar, `Esc` goes back.

## Folder map

```
gui/
├── index.html, package.json, tsconfig.json, vite.config.ts
├── public/sample/            # the static page the viewer shows; it applies the variants the GUI posts
└── src/
    ├── main.tsx, App.tsx     # wiring: MockCore → store; the URL hash; the screen
    ├── core/                 # mirrors crates/dovetail-api: the contract with any core
    │   ├── model.ts          #   Snapshot and its parts (design v2's nouns, snake_case)
    │   ├── protocol.ts       #   Command, Event, ServerMsg
    │   ├── apply.ts          #   the pure reducer (apply.test.ts)
    │   ├── link.ts           #   interface CoreLink { subscribe, send }
    │   ├── mock.ts           #   MockCore: answers Commands with Events, scripted replies
    │   └── fixtures.ts       #   the fake data (project Shop and four others)
    ├── state/
    │   ├── store.ts          #   the Snapshot store and send(); a gap asks for a resync
    │   ├── ui.ts             #   UI-only state: screen, zone, mode, selections, modal …
    │   ├── select.ts         #   derived views (tree rows, visible chat, kebab-case names)
    │   └── urlHash.ts        #   UI state in the URL hash
    ├── keys/
    │   ├── keymap.ts         #   THE key table (a proposal awaiting review)
    │   ├── resolve.ts        #   key → token → sequence → Action (resolve.test.ts)
    │   ├── actions.ts        #   the Action union and the one dispatcher
    │   ├── useKeys.ts        #   the one keydown listener
    │   └── viewport.ts       #   handles on DOM / React Flow the dispatcher needs (scroll, zoom)
    ├── styles/               #   tokens.css (the mock's :root, unchanged) and one file per area
    └── components/           #   shell, home, sidebar, chat, main, canvas, viewer, overlays
```

## Rules that keep it honest

- Components never change the snapshot and never import `core/mock.ts` or `core/fixtures.ts`; they call `send(command)`, and the change comes back as an event folded in by `apply()`.
- UI state (focus, mode, selection, open view) lives in `state/ui.ts`, never in the snapshot.
- Every key is named once, in `keys/keymap.ts`; hints in the UI, the which-key pop-up and the `?` overlay are drawn from it. Mouse handlers call the same `dispatch()` the keys do.
- No timers except MockCore's scripted reply; no `dangerouslySetInnerHTML`; no network requests.

## The app: the real core (ADR 0023)

```bash
npm --prefix gui install
cd gui && npx tauri dev     # the window; its process hosts dovetail-core
```

The Tauri shell is `gui/src-tauri` (crate `dovetail-gui`). At start it serves Home (no project open) from `dovetail_core::home`; opening a project spawns that project's core in the same process. Sessions need the installed `dovetail` binary (`scripts/bootstrap.sh`); without it Home says so and opens nothing. Closing the window stops the core and its sessions; they resume on the next open.

## How the core plugs in

`CoreLink` is the whole contract, in the core's own messages:

```ts
interface CoreLink {
  subscribe(fn: (msg: ServerMsg) => void): () => void;  // a snapshot first, then events with seq
  send(cmd: Command): void;
}
```

`TauriCore` (`core/tauri.ts`) carries them to the Rust core through two Tauri calls, `core_subscribe` and `core_send`; `MockCore` fakes a core in a plain browser. `main.tsx` picks one.

```
components ──UiCommand──► store.send ──translate.ts──► Command ──► CoreLink ──► core
     ▲                        └─ canvas, cards, variants ──► pending.ts (not in the core yet)
     └── view (model.ts) ◄── view.ts ◄── Snapshot ◄── apply.ts ◄── ServerMsg ◄──┘
```

- `core/generated/`: the core's types, generated from `crates/dovetail-api` (`cargo test -p dovetail-api --features ts --test ts_export`). Never edit them; `scripts/check.sh` fails when they are stale.
- `core/apply.ts`: the reducer, a port of the Rust one, run against the same cases (`crates/dovetail-api/tests/fixtures/apply/`).
- `core/view.ts`: the one place that knows both the core's shapes and the view's; components read only the view (`core/model.ts`).
- `core/pending.ts`: what the GUI shows that the core does not serve yet (canvas, cards, variants, reports, Home's extra columns). Empty with the real core; MockCore seeds it with the fake data.
