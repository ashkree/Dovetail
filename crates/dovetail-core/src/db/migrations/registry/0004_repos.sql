-- ADR 0021: a project is a name with zero or more repos and one account. The stored kind goes
-- (no repos is a research-only project), so does the one repo on the project row. Every project
-- is copied; one with a repo_path gets one repos row named after its id (already a slug). A
-- folder project without git keeps a NULL git_common_dir.
CREATE TABLE projects_new (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  account TEXT,
  last_opened_ms INTEGER
);
INSERT INTO projects_new (id, name, account, last_opened_ms)
  SELECT id, name, NULL, last_opened_ms FROM projects;
CREATE TABLE repos (
  id INTEGER PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects_new (id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  git_common_dir TEXT UNIQUE,
  added_ms INTEGER NOT NULL,
  UNIQUE (project_id, name),
  UNIQUE (project_id, path)
);
INSERT INTO repos (project_id, name, path, git_common_dir, added_ms)
  SELECT id, id, repo_path, git_common_dir, COALESCE(last_opened_ms, 0)
  FROM projects WHERE repo_path IS NOT NULL;
DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;
