-- The model and effort the user picked for a session in the model selector (task #39). NULL
-- until they pick one: the session then launches with config.toml's default. Used as `--model`
-- and `--effort` on every later start or resume.
ALTER TABLE agents ADD COLUMN chosen_model TEXT;
ALTER TABLE agents ADD COLUMN chosen_effort TEXT;
