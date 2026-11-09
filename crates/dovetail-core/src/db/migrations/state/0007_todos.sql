-- M11 (task #12): who created each native to-do item. Claude Code's task files carry no
-- attribution (digest §4), so the core maps item → agent from the stream's `TaskCreate`
-- results and keeps the mapping here. `list_id` is `CLAUDE_CODE_TASK_LIST_ID`; `agent_id` is a
-- session's agent or a foreground subagent (`<parent>:<tool_use_id>`).
CREATE TABLE todo_items (
  list_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  PRIMARY KEY (list_id, item_id)
);
