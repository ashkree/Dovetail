-- M7: what reached the user, and what happened. Restored and reconciled at M10.
CREATE TABLE inbox_items (
  id INTEGER PRIMARY KEY,
  agent_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,            -- the API InboxKind as JSON
  blocking INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL,
  resolved_ms INTEGER,
  answer TEXT
);

CREATE TABLE activity (
  id INTEGER PRIMARY KEY,
  at_ms INTEGER NOT NULL,
  kind TEXT NOT NULL,
  agent_id TEXT,
  text TEXT NOT NULL
);

CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  at_ms INTEGER NOT NULL,
  from_agent TEXT NOT NULL,
  to_agent TEXT NOT NULL,
  text TEXT NOT NULL,
  msg_id TEXT
);
