# GUI mock: decisions so far

Running list of what the user settled while iterating on the mock (2026-11-15). Folded into `docs/gui-layout-and-keys.md` at the end.

- Layout: chat on the left, main view in the middle, inbox over the efforts tree on the right.
- Inbox grows downward with its items (up to 40 % of the height); the tree gets the rest.
- Sidebar widens by dragging its edge; collapses to a rail (inbox count, effort dots) with `Space z` or a button.
- All statuses in the bottom bar (Claude-desktop-like): mode, zone, needs-you, efforts count, profile, context, usage, model, permission mode, activity. The top bar holds only the brand, project, repos and view tabs.
- Regions are framed panels with moderately rounded corners; less inside them (no chips, hints, uppercase titles).
- You and the coordinator have distinct colours.
- Messages keep the name, rule, time and effort name visible.
- Text size around 14 px (Claude desktop); 16–18 px rejected as too big.
- Highlight colours in text are softened toward the body text colour.
- Themes: dark neutral grey preferred over Iceberg's deep blue and the deep purple.
- Clickable view tabs (Chat, Canvas, Viewer, Reading) are always visible.
- Effort names are lower kebab case; typed names are converted automatically.
- Mock iteration loop: variants in the design panel, the user keeps a combination, the rest is deleted.
- Kept combination (2026-11-15): Mist on dark grey, highlights 70 %, framed panels, 4 px corners, Atkinson Hyperlegible 14 px, line height 1.5, "where was I" as a pop-up, underlined view tabs, own speaker colours with a bar, chat 440 px, "efforts 5 / 4 ▲". Repos show with a folder icon.
- Folded into `docs/gui-layout-and-keys.md`. The keymap there is still a proposal awaiting its own review.
