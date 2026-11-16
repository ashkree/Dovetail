-- ADR 0025: canvases and cards as core state. Adds only.
-- Numbered 0018 because the contiguity test forbids a gap; the multi-repo work (ADR 0021) also
-- wants 0018 (`0018_agent_repo.sql`), so whichever branch lands second renumbers its file.

-- A canvas of an effort: a board of cards in one framework.
CREATE TABLE canvases (
  id INTEGER PRIMARY KEY,
  effort_id INTEGER NOT NULL REFERENCES efforts(id),
  title TEXT NOT NULL,
  framework TEXT NOT NULL CHECK (framework IN ('question_tree', 'options_compare', 'free_board')),
  created_ms INTEGER NOT NULL
);
CREATE INDEX canvases_by_effort ON canvases (effort_id, id);

-- A card belongs to an effort. `canvas_id` NULL means the incoming strip (x and y are then 0).
-- A card with a reference stores no text: `ref_kind` is finding, source, decision, map or task
-- and `ref_id` the target's id as text (maps have text ids). There is no foreign key on `ref_id`:
-- one column points at five tables, and rows a card can point at are never deleted (ADR 0025,
-- point 4). `added_by` is `user` or an agent id.
CREATE TABLE cards (
  id INTEGER PRIMARY KEY,
  effort_id INTEGER NOT NULL REFERENCES efforts(id),
  canvas_id INTEGER REFERENCES canvases(id),
  kind TEXT NOT NULL CHECK (kind IN ('finding', 'source', 'idea', 'decision', 'question', 'map', 'task')),
  x INTEGER NOT NULL DEFAULT 0,
  y INTEGER NOT NULL DEFAULT 0,
  ref_kind TEXT CHECK (ref_kind IN ('finding', 'source', 'decision', 'map', 'task')),
  ref_id TEXT,
  text TEXT NOT NULL DEFAULT '',
  added_by TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);
CREATE INDEX cards_by_effort ON cards (effort_id, id);
CREATE INDEX cards_by_canvas ON cards (canvas_id);

-- A link from one card to another (a question to the finding that answers it).
CREATE TABLE card_links (
  from_card INTEGER NOT NULL REFERENCES cards(id),
  to_card INTEGER NOT NULL REFERENCES cards(id),
  label TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (from_card, to_card)
);
CREATE INDEX card_links_to ON card_links (to_card);
