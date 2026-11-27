CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  repo_path TEXT NOT NULL,
  git_common_dir TEXT NOT NULL UNIQUE,
  last_opened_ms INTEGER
);
