-- Research follow-ups (decided 2026-11-11, `docs/research.md` + ADR 0015 amendments).
--
-- Round queue: a round requested while another is open is stored at once (`brief.md` written)
-- with `queued = 1` and opens, oldest id first, when the open one closes. `opened_ms` is the
-- request time until it opens. At most one round has `closed_ms IS NULL AND queued = 0`.
ALTER TABLE research_rounds ADD COLUMN queued INTEGER NOT NULL DEFAULT 0;

-- Unfiled findings: research done by the coordinator's subagents with no round open. They have
-- no task until the user files one (a row moves to `findings`, with a new id) or drops it.
-- Same columns as `findings` minus `task_id`/`round_id`/`n`; `path` is relative to the research
-- root's `unfiled/` folder.
CREATE TABLE unfiled_findings (
  id INTEGER PRIMARY KEY,
  agent_id TEXT NOT NULL,
  label TEXT NOT NULL,
  description TEXT,
  path TEXT NOT NULL,                       -- findings/<id>-<label>.md
  body TEXT NOT NULL,
  body_from TEXT NOT NULL,
  subagent_type TEXT,
  started_ms INTEGER,
  created_ms INTEGER NOT NULL,
  tokens INTEGER
);

CREATE TABLE unfiled_finding_sources (
  finding_id INTEGER NOT NULL REFERENCES unfiled_findings(id),
  source_id INTEGER NOT NULL REFERENCES sources(id),
  tool_use_id TEXT NOT NULL,
  PRIMARY KEY (finding_id, source_id, tool_use_id)
);
