-- ADR 0005: a project can be any folder, so the git common dir becomes optional. A folder
-- project is found by its path; UNIQUE still holds for the repos that have one.
CREATE TABLE projects_new (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  repo_path TEXT NOT NULL,
  git_common_dir TEXT UNIQUE,
  last_opened_ms INTEGER
);
INSERT INTO projects_new (id, name, repo_path, git_common_dir, last_opened_ms)
  SELECT id, name, repo_path, git_common_dir, last_opened_ms FROM projects;
DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;
