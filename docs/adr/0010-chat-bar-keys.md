# Chat bar keys: Shift-Enter for a new line, Ctrl-k and Ctrl-j between the bar and the conversation

Refines the "Chat input" paragraph of the design doc (Decision 17's keys for Insert mode) and `docs/ui-session-brief.md`. Before, `Ctrl-j` and `Alt-Enter` started a new line in the chat bar, and the conversation could not be scrolled while typing.

Why: reading a question and writing the answer at the same time meant leaving the chat bar with `Esc`, scrolling, and pressing `i` again; and Shift-Enter, the key Claude Code uses for a new line, sent the message instead.

**Decision:**

- **New line:** `Shift-Enter` where the terminal reports it, `Alt-Enter` everywhere. Dovetail asks the terminal for the kitty keyboard protocol's "disambiguate" flag when the terminal says it supports it, and pops it before leaving the alternate screen (on exit and when lending the terminal to an editor or a take-over). It is never required, as the brief asks: a terminal without it keeps `Alt-Enter`.
- **Ctrl-j no longer types a new line.** It is a focus key, as everywhere else.
- **The chat bar is a stop of its own below the conversation.** `Ctrl-k` in the chat bar goes up to the conversation in Normal mode, keeping the draft, its cursor and its images. `Ctrl-j` (or `i`) in the conversation comes back down to the bar where the cursor was. While the bar is left, its first line stays in view, dimmed, so it is clear what `i` goes back to. `Esc` keeps the draft and cursor the same way.
- **PageUp / PageDown** scroll the conversation by a page (two rows overlap), in Normal mode and while typing in the chat bar.

- **The chat bar is drawn as a pane of its own** under the conversation, in the left column: bordered and titled (`chat › <agent>`) like the inbox and the tree, at most half the column high. While you type it has the focus colour and the conversation pane does not; while you read, the other way round. (First built as a rule inside the conversation pane; changed the same day at the user's request.)

It is still drawn by the conversation buffer rather than being a buffer in a region of its own, so it always sits under the conversation it writes to and Layout B's three regions stay as they are.

Approved by the user in the session of 2026-11-10.
