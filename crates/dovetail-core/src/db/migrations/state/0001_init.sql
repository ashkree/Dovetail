CREATE TABLE tasks (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL,
  p_level INTEGER,
  bucket TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  notes TEXT NOT NULL DEFAULT '',
  pr INTEGER,
  follow_up_of INTEGER REFERENCES tasks(id),
  coordinator_created INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER,
  updated_ms INTEGER
);

CREATE TABLE task_deps (
  task_id INTEGER,
  depends_on INTEGER,
  PRIMARY KEY (task_id, depends_on)
);

-- One row per pair; the API shows the relation both ways.
CREATE TABLE task_related (
  a INTEGER REFERENCES tasks(id),
  b INTEGER REFERENCES tasks(id),
  PRIMARY KEY (a, b),
  CHECK (a < b)
);

CREATE TABLE decisions (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES tasks(id),
  at_ms INTEGER NOT NULL,
  author TEXT NOT NULL,              -- 'user' or an agent id
  text TEXT NOT NULL,
  why TEXT,
  alternatives TEXT NOT NULL DEFAULT '[]',  -- JSON [{option, why_not}]
  supersedes INTEGER REFERENCES decisions(id)
);
