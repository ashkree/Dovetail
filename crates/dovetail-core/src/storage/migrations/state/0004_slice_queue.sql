-- Slice starts over the `max_slices` cap (ADR 0004), kept across restarts (decided 2026-11-06).
-- Oldest first by id.
CREATE TABLE slice_queue (
  id INTEGER PRIMARY KEY,
  task_id INTEGER REFERENCES tasks(id),
  name TEXT NOT NULL,
  brief TEXT NOT NULL,
  by_coordinator INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL
);
