Copied from the Tauri GUI at commit f60519c (full tree on the `gui-shelved` branch).
Not built yet: no `package.json`; `src/core/generated/` is written by `scripts/check.sh` (ts-rs) and kept current.
Built when the page plan lands (ADR 0030 §4); then prune what the page does not use.

## Dangling imports

`src/canvas/*.tsx` still import GUI modules that stayed on `gui-shelved` (paths there under `gui/src/`). The page gets its own store; replace each:

| Imported by | Import | Used for | On `gui-shelved` |
|---|---|---|---|
| `Canvas`, `CardView`, `IncomingStrip` | `dispatch` (`keys/actions`) | run a keymap action (select, open, pick layout, focus zone) | `keys/actions.ts` |
| `Canvas` | `BLOCK_KEY`, `effortBlocks`, `markSeen` (`keys/actions`) | layout key table, an effort's blocks, mark a card seen | `keys/actions.ts` |
| `Canvas` | `viewport` (`keys/viewport`) | canvas viewport handle for keyboard pan/zoom | `keys/viewport.ts` |
| `Canvas`, `CardView` | `keyText` (`components/overlays/Keys`) | show an action's key in hints | `components/overlays/Keys.tsx` |
| `Canvas` | `effortById` (`state/select`) | look up the open effort | `state/select.ts` |
| `Canvas`, `IncomingStrip` | `useSnapshot`, `send` (`state/store`) | read the snapshot, send a command | `state/store.ts` |
| `Canvas`, `IncomingStrip` | `useUi` (`state/ui`) | selected effort, card, incoming | `state/ui.ts` |
