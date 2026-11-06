-- M10: what must outlive a restart besides the agents and the inbox.

-- Signals for the coordinator held while it could not be woken (it crashed, or the app quit);
-- sent when its session next starts, then deleted.
CREATE TABLE coordinator_notes (
  id INTEGER PRIMARY KEY,
  text TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);

-- Worktrees no agent owns that the user chose to keep (`d` on the sweep's item), so the
-- stale-state sweep does not ask about them again. `project_key` is the key the worktree
-- directory sits under.
CREATE TABLE kept_worktrees (
  project_key TEXT NOT NULL,
  name TEXT NOT NULL,
  kept_ms INTEGER NOT NULL,
  PRIMARY KEY (project_key, name)
);
