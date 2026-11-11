-- R2 (docs/research.md §6, ADR 0015): the findings and sources a decision rests on, written when
-- the user accepts a `research_propose` decision. `decisions` itself is unchanged (ADR 0003).
-- `kind` is 'finding' or 'source'; `ref_id` is findings.id or sources.id. No foreign key on
-- `ref_id`: it points at one of two tables.
CREATE TABLE decision_evidence (
  decision_id INTEGER NOT NULL REFERENCES decisions(id),
  kind TEXT NOT NULL,
  ref_id INTEGER NOT NULL,
  PRIMARY KEY (decision_id, kind, ref_id)
);
