# Conversation on the left, tree over inbox on the right

Refines Decision 15 (default layout, Layout B). Only the left/right placement changes: the conversation is now full height on the **left**, and the task tree over the inbox is stacked on the **right**. Stacking (tree over inbox, 60/40), the side column's width rule (about 38 %, at least 40 columns), the 120-column centred reading column and everything else in Decision 15 stay as they were.

**Consequences:**

- Focus keys follow the screen: `h` moves from the side column to the conversation, `l` moves from the conversation back to the side region used last. `Ctrl-j`/`Ctrl-k` still move within the side column. `Action::FocusLeft` and `Action::FocusRight` keep their names and meaning.
- The coordinator chat pane gets extra padding inside its border: 2 columns on each side and 1 row at the top and bottom, from the single constant `theme::CHAT_PADDING`.
- Region names are now `SideTop`, `SideBottom` and `Main`. Buffers are still not tied to regions.
- `docs/screens/` shows the old placement; the screens are reference only and are not redrawn.

Note: the vertical split of the side column was later changed to inbox over tree by ADR 0008.

Approved by the user in the session of 2026-11-06.
