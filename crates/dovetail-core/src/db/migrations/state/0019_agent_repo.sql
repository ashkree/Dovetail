-- ADR 0021: the repo a slice works in, by its registry id (a rename does not break it). NULL for
-- agents from before, and for agents with no repo: read as the project's first repo.
ALTER TABLE agents ADD COLUMN repo_id INTEGER;
-- A queued slice start keeps the repo it was asked for, by name; it is resolved when it starts.
ALTER TABLE slice_queue ADD COLUMN repo TEXT;
