-- M6: sessions and their conversations. Conversation text only, never tool output
-- (Decision 20); a turn's tool activity is stored as its one summary line.
CREATE TABLE agents (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  parent_id TEXT,
  task_id INTEGER REFERENCES tasks(id),
  session_id TEXT,
  transcript_path TEXT,
  task_list_id TEXT,
  worktree_path TEXT,
  branch TEXT,
  setup_state TEXT,
  status TEXT NOT NULL,
  status_line TEXT NOT NULL DEFAULT '',
  permission_mode TEXT NOT NULL,
  cost_usd REAL NOT NULL DEFAULT 0,
  tool_use_id TEXT,
  output_file TEXT,
  created_ms INTEGER,
  last_activity_ms INTEGER,
  closed_ms INTEGER
);

CREATE TABLE conversation_entries (
  id INTEGER PRIMARY KEY,
  agent_id TEXT NOT NULL,
  turn INTEGER,
  at_ms INTEGER NOT NULL,
  kind TEXT NOT NULL,
  text TEXT NOT NULL
);
CREATE INDEX conversation_by_agent ON conversation_entries(agent_id, id);
