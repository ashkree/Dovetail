-- Research-only projects (decided 2026-11-11, ADR 0015's open point): a project has a kind, and
-- a research project has no repo, so repo_path becomes optional too. Every project registered
-- before is a code project. UNIQUE still holds for the git dirs that are set.
CREATE TABLE projects_new (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'code' CHECK (kind IN ('code', 'research')),
  repo_path TEXT,
  git_common_dir TEXT UNIQUE,
  last_opened_ms INTEGER,
  CHECK (kind = 'research' OR repo_path IS NOT NULL),
  CHECK (kind = 'code' OR (repo_path IS NULL AND git_common_dir IS NULL))
);
INSERT INTO projects_new (id, name, kind, repo_path, git_common_dir, last_opened_ms)
  SELECT id, name, 'code', repo_path, git_common_dir, last_opened_ms FROM projects;
DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;
