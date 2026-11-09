-- Task #17 (M11, 2026-11-09): a session's subagent token total, summed from `task_*` events
-- (digest §3: the final `result` undercounts). Kept per session, since finished subagent nodes
-- are pruned. 0 for subagents.
ALTER TABLE agents ADD COLUMN subagent_tokens INTEGER NOT NULL DEFAULT 0;
