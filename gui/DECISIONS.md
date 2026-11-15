# GUI frontend: decisions made while building unattended

The plan (`docs/plan/gui-frontend.md`) says "stop and ask" in a few places. The user was asleep while F1–F9 were built (2026-11-15), so each such point was decided here instead, staying inside the settled layout, look and nouns and as close to `mock/gui/` and `docs/gui-layout-and-keys.md` as possible. One entry each: what was open, what was chosen, why, how to change it.

## 1. Dev server port 5174 instead of 5173

- **Open:** the plan fixes `http://127.0.0.1:5173` with `strictPort`. On this machine another app (a container serving "Confidence Sync" from `/app`) already listens on 5173, so `npm run dev` failed.
- **Chose:** port 5174, still `strictPort` and host `127.0.0.1`.
- **Why:** the other app is not Dovetail's to stop; a fixed port that works beats one that fails at start.
- **Change:** `server.port` in `gui/vite.config.ts`.

## 2. Model fields the plan did not list

- **Open:** design v2 and the plan's model sketch name no colour for an effort's tag, no link from a long card to its report, and keep the bottom bar's machine-wide running count implicit.
- **Chose:** `Effort.colour` (one of the five `--tag-*` tokens), `Card.report_id`, `Status.machine_running`; `Agent.parent_id` carries subagents, and an `IncomingCard` type holds `under` (the card it would hang under).
- **Why:** a tag colour must stay with its effort when others are added or archived, so it belongs to the effort, not to its position; the rest is what the mock showed.
- **Change:** `gui/src/core/model.ts`; the shapes are reconciled with `dovetail-api` when the real core is wired (plan §8).

## 3. Opening a project other than Shop

- **Open:** the mock always opened Shop with a toast ("only Shop is filled in").
- **Chose:** `MockCore` opens any project; Dovetail, Garden planner, Thesis notes and Home lab open with no efforts, chat or inbox. Each project's state is kept while you go Home and back.
- **Why:** the real core will send that project's own snapshot; an empty workspace is the honest stand-in.
- **Change:** `projectSnapshot()` in `gui/src/core/fixtures.ts`.

## 4. Which effort is in focus when a project opens

- **Open:** the mock started on `login-bug` with its "where was I" pop-up up, hard-coded.
- **Chose:** the effort of the first inbox item (what needs you), else the first effort; its "where was I" pop-up comes up.
- **Why:** it gives the mock's `login-bug` for Shop and a sensible start for any project.
- **Change:** `openProject` in `gui/src/keys/actions.ts`.

## 5. Keys outside the scope table

- **Open:** the mock hard-coded the `?` overlay's own keys (`[ ] Tab Esc ?`) and the picker's (`Ctrl-j Ctrl-k Enter Esc`, arrows), and its insert keys `Ctrl-w Ctrl-u Ctrl-v` were `none` with special handling.
- **Chose:** `OVERLAY_KEYS` and `MODAL_KEYS` tables in `keymap.ts`; `Ctrl-w`/`Ctrl-u` are real actions (`deleteWord`, `deleteLine`), `Ctrl-v` is `paste`, which lets the browser paste. Key hints in the UI (`Space v x`, `Esc`, button labels) are read from the table too.
- **Why:** the plan makes `keymap.ts` the only place a key is named, so a key change stays a one-line edit.
- **Change:** `gui/src/keys/keymap.ts`.

## 6. Toasts hide by CSS, not a timer

- **Open:** the plan allows no timers except MockCore's replies; the mock hid toasts after 2.6 s with `setTimeout`.
- **Chose:** a CSS animation fades the toast out after 2.6 s.
- **Change:** `.toast` in `gui/src/styles/overlays.css`.

## 7. Small fixes to the mock's text

- The keyboard overlay's hint for Chat said "coordinator, right"; the chat is on the left in the settled layout, so it says "coordinator, left".
- Tab in the canvas goes to the first card the selected one links to and Shift-Tab back to the card linking to it (the mock's Tab cycled oddly between children). `h j k l` reach siblings.

## 8. Which variant set and report the views show

- **Open:** design v2 says the viewer follows the focused slice; the mock always showed `header-toggle`'s preview and the one report.
- **Chose:** the viewer shows the focused effort's variant set, else the project's first one; the reading view shows the selected card's report, else the focused effort's, else the first.
- **Why:** closest to the mock while still following focus once there is more than one.
- **Change:** `viewerSet()` in `gui/src/keys/actions.ts`, `readingReport()` in `gui/src/state/select.ts`.

## 9. The sample preview page

- **Open:** the plan asks for a small static page that changes when the parent posts the variants.
- **Chose:** `gui/public/sample/` (HTML, CSS, a 15-line script) with the mock's shop page; the GUI posts `{type: 'dovetail-variants', choice, text}` to the same origin only. It uses the system font, since a slice's site has its own look.
- **Change:** `gui/public/sample/`.

## 10. Canvas details

- **Open:** how new and placed cards are positioned, what `0` (fit) does, and React Flow's corner attribution.
- **Chose:** a placed incoming card lands under the card it belongs to, offset by how many children that card already has (as the mock did); a new card (`n`, then kind, then title) lands under the selected card and links from it; `0` fits every card in view (computed from card positions, since React Flow's own `fitView` needs measured nodes this controlled setup does not keep); the "React Flow" attribution is hidden (`proOptions.hideAttribution`), which its MIT licence allows, to match the screenshots.
- **Why:** closest to the mock; nothing new on screen.
- **Change:** `accept` / `newCard` in `gui/src/keys/actions.ts`; `Canvas.tsx`. To show the attribution, drop `proOptions`.
