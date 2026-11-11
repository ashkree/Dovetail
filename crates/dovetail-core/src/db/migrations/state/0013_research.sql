-- R1 (docs/research.md §3, ADR 0015): research rounds, findings and the sources they cite.
-- Paths are relative to the task's research folder (`research/<task-id>-<slug>/`). The database
-- is the source of truth; the files are a readable copy the core rebuilds when missing, so
-- every file's content is here too (`brief`, `findings.body`, `sources.content`).
CREATE TABLE research_rounds (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES tasks(id),
  n INTEGER NOT NULL,                       -- 1-based per task
  opened_ms INTEGER NOT NULL,
  closed_ms INTEGER,                        -- NULL while open; at most one round is open
  brief TEXT NOT NULL,                      -- the user's criteria, also written to brief.md
  dir TEXT NOT NULL,                        -- rounds/01-2026-11-10
  UNIQUE (task_id, n)
);

CREATE TABLE findings (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES tasks(id),
  round_id INTEGER REFERENCES research_rounds(id),  -- NULL: quick finding, no round
  n INTEGER NOT NULL,                       -- 1-based within the round (or the task's quick ones)
  agent_id TEXT NOT NULL,                   -- agents.id of the subagent
  label TEXT NOT NULL,                      -- slug used in the file name
  description TEXT,                         -- the Agent call's description
  path TEXT NOT NULL,                       -- rounds/01-2026-11-10/findings/01-<label>.md
  body TEXT NOT NULL,                       -- the full report; the file adds a header
  body_from TEXT NOT NULL,                  -- which stream field the body came from
  subagent_type TEXT,
  started_ms INTEGER,
  created_ms INTEGER NOT NULL,
  tokens INTEGER
);
CREATE INDEX findings_task ON findings (task_id);

CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,                       -- 'web' | 'file' | 'search'
  locator TEXT NOT NULL,                    -- URL, absolute file path, or search query
  tool TEXT NOT NULL,                       -- 'WebFetch' | 'WebSearch' | 'Read' | 'Grep' | 'Glob'
  query TEXT,                               -- WebFetch's prompt, WebSearch's query
  content TEXT NOT NULL,                    -- what the tool returned to the agent
  cached INTEGER NOT NULL,                  -- 1: written to sources/<id>.md (Grep/Glob: 0)
  is_error INTEGER NOT NULL DEFAULT 0,
  content_hash TEXT NOT NULL,               -- dedups the same page fetched twice
  fetched_ms INTEGER NOT NULL,
  UNIQUE (locator, content_hash)
);

CREATE TABLE finding_sources (
  finding_id INTEGER NOT NULL REFERENCES findings(id),
  source_id INTEGER NOT NULL REFERENCES sources(id),
  tool_use_id TEXT NOT NULL,                -- the stream's tool_use id, for audit
  PRIMARY KEY (finding_id, source_id, tool_use_id)
);

-- Written by `research_write_report` (R2); here so R2 needs no migration of its own for it.
CREATE TABLE reports (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES tasks(id),
  round_id INTEGER REFERENCES research_rounds(id),
  at_ms INTEGER NOT NULL,
  body TEXT NOT NULL,
  uncited INTEGER NOT NULL DEFAULT 0,
  unlabelled_figures INTEGER NOT NULL DEFAULT 0
);

ALTER TABLE tasks ADD COLUMN research_kind TEXT;   -- NULL | 'question' | 'decision'
ALTER TABLE tasks ADD COLUMN altitude TEXT;        -- NULL | 'macro' | 'micro'
