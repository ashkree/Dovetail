# The core runs each slice's preview; a slice declares its variants with a tool and the page applies them

**Status: Accepted.** Decided by the user on 2026-11-16. Built: see `docs/plan/viewer-preview.md` (state migration `0020_design_variants.sql`, the `design` tool suite, the preview runner in the core, the viewer reading both from the core).

Settles open point 3 of `docs/design-v2.md` ("how variants are declared") and the "preview command" of its section 7. The look and keys of the viewer (`docs/gui-layout-and-keys.md`) do not change.

## Decision

**1. One recipe line starts a slice's preview.** A repo's recipe gets `preview`, a shell command, at the project level and overridable in `[repos.<name>]` like `setup` (ADR 0021 point 6):

```toml
preview = "npm run dev -- --port {port}"
[repos.web]
preview = "python3 -m http.server {port} --bind 127.0.0.1"
```

`{port}` is replaced with a free port on 127.0.0.1 that the core picks. The core starts the command the first time the viewer is opened on that slice (the GUI sends `StartPreview`), in the slice's worktree, with the recipe's `[env]`, inside a memory-capped scope with the same cap as the slice's session. It stops it when the slice is archived and when the app quits. A restart command stops and starts it again. Nothing is written into the repo.

**2. The preview's state is core state, per slice, in the snapshot.** `NotStarted`, `NoCommand` (the recipe has no `preview` for this repo), `Starting`, `Running { url }`, `Failed { output }` (the last lines it printed, or why it never listened). It lives only in memory: a process does not survive the app. "Running" means something accepted a connection on the port; the core checks that while starting, not afterwards (no idle polling). A process that exits later turns the state to `Failed` with its last output.

**3. A slice declares its variants with `design_declare_variants`.** A new tool suite `design`, in the slice role's default suites (and in `Suite::ALL`). The call replaces the slice's whole list. A slot has a `name` (its key, unique), a `level` (`global`, `component` or `content`), its `options` (strings, at least one) and optionally the `kept` option. Each call is checked as a whole: at most 12 slots, 8 options each, names and options non-empty and unique. The list and the user's selection are core state per slice (`design_variants` in `state.db`), travel in the snapshot, and survive a restart.

**4. The selection is a map from slot to option.** The user's pick is a command (`SelectVariant { agent, slot, option }`); the core stores it and announces `VariantsUpserted`. A slot with no pick shows its kept option, else its first. A new declaration keeps a pick that is still a valid option of a slot of the same name.

**5. "Keep this combination" is a prompt to the slice.** `KeepVariants { agent }` sends the slice a prompt naming the chosen options and asking it to make them permanent, delete the other options and the variant code, and call `design_declare_variants` again with what is left (an empty list when nothing is). The core marks the chosen options as kept at once.

**6. The page-side contract (what the slice writes).** The viewer sends the selection to the previewed page with `postMessage`, to the iframe's window, on load and on every change:

```js
{ dovetailVariants: { "<slot name>": "<option>", ... } }   // every declared slot, always
```

The page listens for it and applies it, for example by setting a `data-<slot>` attribute on `<html>` or swapping a component:

```js
addEventListener('message', e => {
  const v = e.data && e.data.dovetailVariants;
  if (v) for (const [slot, option] of Object.entries(v)) document.documentElement.dataset[slot] = option;
});
```

Nothing else is injected into the page: no script tag, no proxy, no query string. The page is the slice's code, so the slice owns the listener and removes it when it keeps a combination. The message targets the preview's origin only. Keys are the slot names exactly as declared; values are option strings exactly as declared.

**7. Where this lives.** The slice role prompt (`config/roles/slice.md`) carries the three-line snippet; the tool's description carries the longer explanation (the levels, the contract, the keep flow). The terminal UI shows nothing new.

## Choices where the decision is silent

- **Slot name is the key**, not a separate id: the agent writes it and the page reads it.
- **Selection stored fully resolved** (every slot has a pick), so the viewer never computes defaults.
- **The preview starts only for slices that have a worktree**; other agents report `Failed`.
- **Readiness is "accepts a TCP connection on the port"**, checked every 200 ms for at most 60 s after the start, never afterwards.
- **A command that exits before listening is `Failed`**; the output tail is the last 20 lines of stdout and stderr together.
- **A free port is found by binding port 0 and releasing it**; the small window before the command binds it is accepted.
