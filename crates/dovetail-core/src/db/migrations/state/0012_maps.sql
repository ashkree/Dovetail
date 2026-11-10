-- M15 (task #35): maps drawn with `draw_map`, per project, each version kept (design 23h).
-- `maps` is the shelf row, describing the latest version; `map_versions` holds every version's
-- validated model (`dovetail_api::map::MapContent`) as JSON. A map keeps its id and type across
-- versions. `task_id` is the task of the agent that first drew it (NULL for the coordinator);
-- `drawn_by` is the agent id that drew the latest version, or that version.
CREATE TABLE maps (
  id TEXT PRIMARY KEY,
  task_id INTEGER REFERENCES tasks(id),
  title TEXT NOT NULL,
  type TEXT NOT NULL,
  current_version INTEGER NOT NULL,
  drawn_by TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);
CREATE TABLE map_versions (
  map_id TEXT NOT NULL REFERENCES maps(id),
  version INTEGER NOT NULL,
  model TEXT NOT NULL,
  drawn_by TEXT,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (map_id, version)
);
