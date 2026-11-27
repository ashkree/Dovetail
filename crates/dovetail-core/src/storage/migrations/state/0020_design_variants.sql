-- ADR 0026: the variants a slice declared for its web preview and the user's pick in each.
-- `slots` and `selection` are JSON (`Vec<VariantSlot>`, `{slot: option}`). One row per slice;
-- deleted with the slice by the core.
CREATE TABLE design_variants (
    agent_id  TEXT PRIMARY KEY,
    slots     TEXT NOT NULL,
    selection TEXT NOT NULL
);
