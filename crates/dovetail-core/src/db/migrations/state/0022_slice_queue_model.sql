-- Task #42: a queued slice start keeps the model and effort the coordinator asked for, so the
-- session starts on them when its turn comes. NULL: the default.
ALTER TABLE slice_queue ADD COLUMN model TEXT;
ALTER TABLE slice_queue ADD COLUMN effort TEXT;
