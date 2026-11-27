-- The last model list a driver reported (agy: `agy models`, or a session's own list), kept so
-- the model picker has the driver's rows at start without waiting for it. One row per driver;
-- `models` is a JSON array.
CREATE TABLE driver_models (
    driver   TEXT PRIMARY KEY,
    models   TEXT NOT NULL,
    saved_ms INTEGER NOT NULL
);
