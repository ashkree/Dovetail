# GUI performance (task #29)

User report (2026-11-17): the whole GUI is slow and not crisp. Measured first, then fixed.

## Method

`scripts/perf-gui.sh` builds the window with a perf harness (`VITE_PERF=1`, `--features perf`) and runs it once in the real WebKitGTK window on the user's machine. The harness (`gui/src/perf/`) swaps in a synthetic core holding what a few hours of work leave behind: a coordinator chat of 300 entries with markdown (lists, code, a table), 300 entries in each of the 7 slice conversations, 40 cards with links on the canvas (427 KB of snapshot JSON). It presses keys through the real `keydown` handler, streams `assistant_delta` events, zooms the canvas with wheel events, and reports through one shell command (`perf_log`, only in a perf build, with its own capability). Counted per scenario: time from the key to the end of React's work (`sync`) and to just after the next frame (`paint`), frames per second, React commits, and which components rendered (a stand-in for the DevTools hook walks the fiber tree). The script also samples CPU and PSS of the app and its WebKit processes while idle.

The release build used here has LTO off (`CARGO_PROFILE_RELEASE_LTO=off`): the session's 3 GB memory cap kills the thin-LTO link. The core crates do not matter for these numbers (frontend only).

## Before and after

Release build, 300-entry chat, 40 cards. Milliseconds per action, mean of 20-40, one run each on a synthetic core (the real core's IPC is not in these numbers). The like-for-like figure is paint: 127-141 ms before (256 ms for the Space leader), 17-30 ms after; `sync` is the time to the end of React's work.

| Scenario | Before (sync / paint) | After (sync / paint) |
|---|---|---|
| `j`/`k` in the tree | 121 / 141 (p95 155) | 3.1 / 16.7 (p95 18) |
| `Space` leader, then `Esc` | 120 / 256 (p95 301) | 2.7 / 18.7 (p95 32) |
| `i` / `Esc` (mode change) | 119 / 129 | 2.8 / 30 (p95 35) |
| view switch canvas / viewer | 120 / 127 | 9.6 / 24.6 (p95 46) |
| typing in the chat bar | 118 / 129 | 2.6 / 18.4 |
| streaming 30 deltas/s, 5 s | 3.9 fps, worst frame 274 ms | 62 fps, worst 24 ms |
| streaming 100 deltas/s, 5 s | 3.8 fps, worst frame 281 ms | 61 fps, worst 24 ms |
| canvas zoom step | 1.4 / 17.6 | 1.5 / 17.3 |
| canvas wheel zoom, 3 s | 62 fps | 62 fps |
| idle, a turn running (spinner) | 26 % of a core (web process) | 2 % |
| idle, nothing running | 0 % | 0 % |

A development-mode React bundle gave the same numbers within noise.

## Hot spots, in order of impact

1. **Every UI change re-rendered the whole chat.** `Message` was not memoised and called `useSnapshot()`; every key press, mode change and tooltip went through `useUi()` in nearly every component, so ~270 messages re-rendered per commit and their markdown was parsed again (the cost was ~110 ms per key). Fix: `Message`/`ReportLine` take plain props (`shown`: speaker name and tag) and are memoised with a field comparison; `Markdown` is memoised on its text. Same for the canvas: `CardView` and the React Flow node component are memoised, `viewCanvas` is cached on the data it is made from, and the node list keeps a card's node while its card, selection and position are unchanged (a selection move rebuilds two nodes, not forty).
2. **One render per streamed delta.** Each `assistant_delta` was folded and notified on its own (and copied the conversation list). Fix: deltas queue and are folded once per frame (`applyBatch`, which also joins consecutive deltas for the same entry; 50 ms timer when the window is hidden). Anything else flushes the queue first, so order is kept. Tested in `core/batch.test.ts`.
3. **The "a turn is running" spinner cost 26 % of a core.** A CSS animation, of any kind and rate (transform, steps, opacity were all tried), makes WebKitGTK redo the page's rendering every frame; the cost grows with the size of the page. It now turns a quarter every 250 ms from a small component state (see `gui/DECISIONS.md` 13).

## Looked at, not the problem

- Store and `toView`: 0.35-0.55 ms for the whole snapshot. Replacing the whole snapshot per event is fine once rendering is cheap.
- IPC: events are small JSON on a Tauri channel; a full snapshot goes out only on subscribe, resync and gap. Not measured separately, since React was two orders of magnitude above it; the harness drives the store directly.
- CSS: no `backdrop-filter`, no transitions; big blurred shadows exist only on transient overlays (which-key, modal, toast, detached panel).
- WebKitGTK compositing: `WEBKIT_DISABLE_COMPOSITING_MODE=1` and `WEBKIT_DISABLE_DMABUF_RENDERER=1` did not make key handling faster and raised memory (web process PSS 660 MB and 581 MB against 365 MB). Left at the default. (`MESA-EGL: failed to create dri2 screen` is printed on this machine either way.)
- Canvas pan/zoom with 40 cards: 62 fps before and after.

## Memory (design v2 open point 11, 500 MB budget)

PSS with the synthetic data: web process 310-370 MB, app (window shell plus in-process core) 123-141 MB, network process 27 MB: about 510 MB, at the budget, with a 300-message chat and 7 more conversations in memory. Not reduced here. Candidates: window the chat DOM (render the last N entries), keep fewer conversation entries client-side than the core's 2000.

## Left

- The chat still renders every entry (up to 2000); memoised, so cheap per frame, but its DOM drives the web process's memory. A windowed list is a feature-sized change.
- React Flow's `NodeWrapper`/`EdgeWrapper` still render on each commit while the canvas is open (about 50 per commit); they are cheap (sync time is 3 ms) and come from inside the library.
- Components still read the whole UI store (`useUi()`); fine now that the leaves are memoised, worth a selector hook if the component count grows.
- Real-core IPC and a real session's stream rate were not measured (no real sessions run for this task).
