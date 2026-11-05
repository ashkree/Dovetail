# Coordinator starts slices with `start_slice`

Refines Decision 25 and plan item B1, under which only the user starts slices (`<leader>sn`) and the coordinator only recommends them. Parallel work then waits on the user for every slice, which defeats the coordinator splitting a feature. S2 passed (an idle slice wakes on `SendMessage`), which was B1's condition for this upgrade.

The coordinator gets an MCP tool `start_slice { task, name, brief }`, shipped in wave 2 with the other tools (M9):

- **Approval follows the coordinator's permission mode.** In auto mode the slice starts with no prompt; in manual mode (and any mode that asks) it raises a Permission item in the inbox, answered with `y`/`d`/`A`.
- **Existing tasks only.** `task` must name a task; for new work the coordinator calls `create_task` first.
- **Capped.** `max_slices` in `config.toml`, default 3. A call over the cap is queued and starts when a slice is archived.
- **Returns at once** with `started`, `queued` or `denied`. The brief becomes the slice's first prompt after setup, exactly as for `<leader>sn`.
- **Ready signal.** When the slice is ready, or its setup fails, Dovetail sends the coordinator a short user message (`slice <name> ready`, `setup failed: …`), so it knows when `SendMessage` will reach the slice.
- Slices cannot call it.

Approved by the user in the session of 2026-11-05 that planned waves 1 and 2.
