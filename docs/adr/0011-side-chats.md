# Side chats: a light session for questions and research

Refines Decision 18 (side-chat helper, v1.1) and the glossary's list of agents. Decision 18 planned a lightweight read-only session that starts on its own during a grill. This brings the general form forward into v1: a session the user starts for a question or some research, apart from the coordinator's work, without the worktree and branch a slice costs.

Why: the user wants to ask about something else now and then and talk to that session directly, and slices are too heavy for it (2026-11-10).

**Decision:**

- **A fourth agent kind, `SideChat`** (glossary: *Side chat*). A Claude Code session like the coordinator and slices, shown in the tree with the other agents that have no task, in the switcher and the finder.
- **Started by the user:** `<leader>sq` or `:ask [question]`. The question is its first prompt; the conversation opens in the main region. It is named `side-N`.
- **No worktree, no task.** It runs in the main checkout (the project folder) and does not count towards `max_slices`.
- **Read-only.** `Edit`, `Write` and `NotebookEdit` are disallowed (`--disallowedTools`). `WebSearch` and `WebFetch` are allowed without asking. Bash follows the session's permission mode like any other session: in manual mode each command asks.
- **Its own role prompt**, `config/roles/side-chat.md`, and Dovetail's tools as a slice gets them (`ask_user`, `report_status`, messaging); `log_decision` needs a task, so a side chat must name one, as the coordinator does.
- **A faster model by default:** `side_chat_model` in `config.toml`, default `sonnet`.
- **Fresh by default:** it does not see the coordinator's conversation.
- **Closed with `dd`** on its tree row: the session stops and the agent is forgotten. It is resumed on restart like a slice until then.
- **`<leader>y`** copies the side chat's last answer into the coordinator's chat bar (as Decision 18 planned).

Decision 18's automatic start on a `grill*` Skill call, and `<leader>?` swapping the side chat into the inbox region, stay v1.1 and reuse this kind.

Approved by the user in the session of 2026-11-10 (name "Side chat", fresh context by default).

**Amended 2026-11-11 (grill with the user; built in PR #52, hardened in PR #58).** Four changes to the decision above:

- **`<leader>y` is removed** (`Action::YankToCoordinator`, its binding and its docs lines). An answer reaches the coordinator through the user's own words, or through ADR 0017's destinations (attach to the current task as a finding, or Tangents).
- **Read-only is real.** A side chat launches with `--permission-mode dontAsk`, so any tool use not pre-approved is denied without asking; "Bash follows the session's mode" no longer holds. Bash is allowed only for a read-only list (`SIDE_CHAT_BASH` in `session/launch.rs`: `git log/show/diff/status/blame`, `rg`, `grep`, `ls`, `cat`, `head`, `tail`, `wc`, `find`, `cargo tree --locked`, `cargo metadata --locked`), plus `WebSearch` and `WebFetch`; `--disallowedTools` holds `Edit`, `Write`, `NotebookEdit` and the forms of `find`, `rg` and `git` that write or run commands. The mode never cycles (`<leader>sm` is refused) and the tree shows `read-only`. `cargo` takes `--locked` so a stale `Cargo.lock` is never rewritten (PR #58).
- **Side chats live in the side panel.** `<leader>?` toggles it (ADR 0017) and `<leader>sq` and `:ask` open the new side chat there, not in the main region.
- **Core suite only**, whatever `[suites]` says: a longer `side_chat` list is ignored with a status-bar warning (ADR 0016; PR #58). This also replaces "Dovetail's tools as a slice gets them": a side chat has `core_ask_user`, `core_report_status` and messaging only, so it has no `core_log_decision` and no `maps_draw`.

Decision 18's `<leader>?` swap into the inbox region is superseded by the panel taking the task tree's region (PR #52).
