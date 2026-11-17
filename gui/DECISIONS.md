# GUI frontend: decisions made while building unattended

The plan (`docs/plan/gui-frontend.md`) says "stop and ask" in a few places. The user was asleep while F1–F9 were built (2026-11-15), so each such point was decided here instead, staying inside the settled layout, look and nouns and as close to `mock/gui/` and `docs/gui-layout-and-keys.md` as possible. One entry each: what was open, what was chosen, why, how to change it.

## 1. Dev server port 47815 instead of 5173

- **Open:** the plan fixes `http://127.0.0.1:5173` with `strictPort`. On this machine another app already listens on 5173, so `npm run dev` failed. 5174 (the first choice) is Vite's own fallback and also often taken.
- **Chose:** port 47815 (user's call, 2026-11-16: a port not commonly taken), still `strictPort` and host `127.0.0.1`.
- **Why:** a fixed port that works beats one that fails at start; the Tauri window loads the same URL.
- **Change:** `server.port` in `gui/vite.config.ts` and `build.devUrl` in `gui/src-tauri/tauri.conf.json` together; they must hold the same value.

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

## 11. Actions the mock only sketched

- **Open:** several keys in the table did nothing real in the mock (new project, add repo, reorder, archive, search, open an agent, new slice, interrupt, permission mode).
- **Chose:** they keep the mock's behaviour: a toast saying "not in this build", or for new project the mock's sketch form. New effort, rename effort and add task are real (they send `new_effort`, `rename_effort`, `add_task`), because the plan lists those commands; new effort asks name → kind → question or goal in three prompts.
- **Why:** the plan's out-of-scope list and command list.
- **Since the live smoke test (2026-11-16):** `Space s m` is real: it sends `cycle_permission_mode` for the coordinator (auto → acceptEdits → manual), since without it no permission request could be produced. See `docs/plan/smoke-test.md`. Since 2026-11-17 it, and `Shift-Tab` in the chat and the tree, act on the agent under the tree cursor when the tree has focus (`docs/plan/smoke-decisions.md`, step 4); `Space s n` is gone (only the coordinator starts slices).
- **Change:** `gui/src/keys/actions.ts`.

## 12. Build size warning

- **Open:** Vite warns when a bundle passes 500 kB; React Flow and the markdown stack make ours ~650 kB.
- **Chose:** raise the warning limit to 900 kB rather than split the bundle.
- **Why:** it is one local bundle loaded from disk by a desktop app, not a page on the web.
- **Change:** `build.chunkSizeWarningLimit` in `gui/vite.config.ts`.

## 13. Projects and repos from Home (ADR 0021)

- **Open:** `docs/gui-layout-and-keys.md` binds `n` (new project), `r` and `Space p r` (add repo) and `Space s n` (new slice) but does not say what each asks.
- **Chose:** one form modal in the picker's style: a label and an input per field, `Tab`/`Shift-Tab` (or `Ctrl-j`/`Ctrl-k`, arrows) between fields, `Enter` sends, `Esc` cancels; the first field is required. New project asks Name, Claude account (empty: default) and First repo (a folder in a git repo; empty: research only). Add repo asks Folder and Name (empty: the folder's name) and acts on the selected project on Home, the open one in a workspace. (New slice, which asked for the repo, Name and Brief, went on 2026-11-17: only the coordinator starts slices. `x` on Home removes a repo: a picker of the project's repos, then a keep/remove picker as the confirm.) The project list's note line went: the core has no such field.
- **Why:** closest to what is settled: the existing picker and prompt modals; the repo picker is the "asks for the repo" the brief named.
- **Change:** `gui/src/keys/actions.ts`, `gui/src/components/overlays/Picker.tsx`, `gui/src/keys/useKeys.ts`, `MODAL_KEYS` in `gui/src/keys/keymap.ts`.

## 14. Agent conversations, interrupt, archive, reorder (2026-11-17, `docs/plan/usable-v2.md`)

- **Open:** the layout doc said "not mocked yet: an agent's own conversation"; the brief asked for it, plus interrupt, archive and reorder, and to stay close to the mock where the doc is silent.
- **Chose:**
  - A main view `agent`, built from the chat's parts: the same messages (the agent in the coordinator's colour, since both are agents), a header line (name, its slice for a subagent, branch, state, permission mode), tool summaries as one dim `⚙` line (the coordinator chat still hides them), the agent's open items in framed boxes above its own chat bar (the first one expanded and answered by `y A d D r x`), the bar saying `to <name>` and the effort dot of its task. Its top-bar tab is named after the agent and shows only while it is open, like `Inbox item`. It has its own draft, so a half-typed coordinator message survives.
  - Keys: tree `c` (any agent, or a task's slice), `Enter` on a subagent, chat `Enter` on a report line, canvas `ga`, the inbox item's `Enter`, the picker. Tree `Enter` on a slice still opens its preview (smoke decision 1).
  - `Space s x` and `Shift-Tab` act on the agent view's agent when it has focus, the tree cursor's next, else the coordinator; a subagent goes to its slice (it runs in that session).
  - `dd` asks with the keep/act picker the remove-repo flow uses. A task is archived (the core ends its slices); an effort is closed (`update_effort` status closed; the core refuses Unsorted); a slice is ended (`close_slice`, worktree and branch stay); a subagent is refused.
  - `J`/`K` move a task among its effort's tasks: the core orders a whole bucket, so the GUI sends the delta that passes exactly one task of the same effort. The tree now lists tasks in the core's order. Efforts have no order in the core and do not move.
- **Why:** nothing new on screen beyond what the chat and inbox already look like; every command already existed in the core.
- **Change:** `gui/src/components/main/AgentView.tsx`, `gui/src/keys/actions.ts` (`showAgent`, `agentInFocus`, `modeTarget`, `archive`, `moveDown`), `gui/src/core/translate.ts` (`reorderDelta`), `gui/src/keys/keymap.ts`.

