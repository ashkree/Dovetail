# Inbox over tree in the side column

Refines ADR 0006 (and through it Decision 15, Layout B) on placement only. The side column on the right now stacks the **inbox on top** (40 %) and the **task tree below** (60 %). Before, it was tree over inbox, 60/40. The side column's width rule (about 38 %, at least 40 columns), the conversation full height on the left, the 120-column centred reading column and the chat padding stay as they were.

Why: what needs you is the first thing to read, so it goes at the top of the column (task #14).

**Consequences:**

- Regions keep their names, `SideTop`, `SideBottom` and `Main`; only the default buffer in each side region changes (`SideTop` shows the inbox, `SideBottom` the tree). Buffers are still not tied to regions: jumping to the inbox (`␣n`, `:inbox`, `Esc` from an inbox item, the finder) focuses whichever side region shows it.
- Focus keys follow the screen: `Ctrl-k` moves up to the inbox, `Ctrl-j` down to the tree, and `l` from the conversation returns to the side region used last.
- Focus still starts on the tree (now `SideBottom`), so `j`/`k`, `Enter` and `i` act on tasks and agents at startup as before.
- `docs/screens/` and the ASCII sketches in the docs show the old stacking; they are reference only and are not redrawn.

Approved by the user in the session of 2026-11-09 (task #14).
