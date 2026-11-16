-- v2 (docs/design-v2.md §3, §4, §10): efforts, their acceptance criteria, and the effort tag.
-- Adds only: no existing table is rebuilt and no row is rewritten except to fill the new columns.

-- One line of work. `name` is lower kebab case, unique in the project. `where_was_i` is the
-- "where was I" card as JSON of `dovetail_api::WhereWasI` ({last_done, waits_on_you, next_step}).
CREATE TABLE efforts (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('feature', 'fix', 'cleanup', 'research', 'experiment')),
  question_or_goal TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
  where_was_i TEXT NOT NULL DEFAULT '{}',
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);

-- An acceptance criterion. `claimed`: an agent says it is met, with `evidence`, and it waits for
-- the user. `met` is written only by the user's approval.
CREATE TABLE criteria (
  id INTEGER PRIMARY KEY,
  effort_id INTEGER NOT NULL REFERENCES efforts(id),
  text TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'claimed', 'met')),
  evidence TEXT,
  created_ms INTEGER NOT NULL
);
CREATE INDEX criteria_by_effort ON criteria (effort_id, id);

-- Every project starts with Unsorted (`dovetail_api::UNSORTED_EFFORT`, id 1), which the user
-- sorts by hand. No kind fits it; `cleanup` is the nearest.
INSERT INTO efforts (id, name, kind, question_or_goal, created_ms, updated_ms)
  VALUES (1, 'unsorted', 'cleanup', 'Tasks not sorted into an effort yet',
          CAST(strftime('%s', 'now') AS INTEGER) * 1000,
          CAST(strftime('%s', 'now') AS INTEGER) * 1000);

-- Every task belongs to an effort. SQLite cannot add a NOT NULL column that references another
-- table, so the column is nullable and the code always writes it; existing tasks go to Unsorted.
ALTER TABLE tasks ADD COLUMN effort_id INTEGER REFERENCES efforts(id);
UPDATE tasks SET effort_id = 1;
-- The criterion a task serves; NULL flags a task that serves none.
ALTER TABLE tasks ADD COLUMN criterion_id INTEGER REFERENCES criteria(id);

-- The effort tag of a chat line; NULL for lines from before efforts and for untagged ones.
ALTER TABLE conversation_entries ADD COLUMN effort_id INTEGER REFERENCES efforts(id);
