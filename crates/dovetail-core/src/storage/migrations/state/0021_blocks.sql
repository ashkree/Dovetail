-- ADR 0027: one canvas per effort; a canvas row becomes a block on it, with a position. Cards
-- keep their positions; `canvas_id` becomes `block_id`; `free_board` becomes `free_area`; a card
-- an agent puts into a block is unseen until the user selects it (existing cards are seen).
-- Tables are rebuilt (a CHECK cannot be altered), children first so foreign keys stay valid.

CREATE TABLE blocks (
  id INTEGER PRIMARY KEY,
  effort_id INTEGER NOT NULL REFERENCES efforts(id),
  title TEXT NOT NULL,
  layout TEXT NOT NULL CHECK (layout IN ('question_tree', 'options_compare', 'free_area')),
  x INTEGER NOT NULL DEFAULT 0,
  y INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL
);
-- Old canvases each held their cards anywhere; a block starts where its first card is.
INSERT INTO blocks (id, effort_id, title, layout, x, y, created_ms)
  SELECT c.id, c.effort_id, c.title,
         CASE c.framework WHEN 'free_board' THEN 'free_area' ELSE c.framework END,
         COALESCE((SELECT MIN(x) FROM cards WHERE canvas_id = c.id) - 20, 0),
         COALESCE((SELECT MIN(y) FROM cards WHERE canvas_id = c.id) - 40, 0),
         c.created_ms
  FROM canvases c;

CREATE TABLE cards_new (
  id INTEGER PRIMARY KEY,
  effort_id INTEGER NOT NULL REFERENCES efforts(id),
  block_id INTEGER REFERENCES blocks(id),
  kind TEXT NOT NULL CHECK (kind IN ('finding', 'source', 'idea', 'decision', 'question', 'map', 'task')),
  x INTEGER NOT NULL DEFAULT 0,
  y INTEGER NOT NULL DEFAULT 0,
  ref_kind TEXT CHECK (ref_kind IN ('finding', 'source', 'decision', 'map', 'task')),
  ref_id TEXT,
  text TEXT NOT NULL DEFAULT '',
  added_by TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  seen INTEGER NOT NULL DEFAULT 1
);
INSERT INTO cards_new (id, effort_id, block_id, kind, x, y, ref_kind, ref_id, text, added_by, created_ms)
  SELECT id, effort_id, canvas_id, kind, x, y, ref_kind, ref_id, text, added_by, created_ms FROM cards;

CREATE TABLE card_links_new (
  from_card INTEGER NOT NULL REFERENCES cards_new(id),
  to_card INTEGER NOT NULL REFERENCES cards_new(id),
  label TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (from_card, to_card)
);
INSERT INTO card_links_new SELECT from_card, to_card, label FROM card_links;

DROP TABLE card_links;
DROP TABLE cards;
DROP TABLE canvases;
ALTER TABLE cards_new RENAME TO cards;
ALTER TABLE card_links_new RENAME TO card_links;

CREATE INDEX blocks_by_effort ON blocks (effort_id, id);
CREATE INDEX cards_by_effort ON cards (effort_id, id);
CREATE INDEX cards_by_block ON cards (block_id);
CREATE INDEX card_links_to ON card_links (to_card);
