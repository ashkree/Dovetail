-- Task #11 (M11a, 2026-11-09): subagents as tree nodes. A subagent's progress (description, last
-- tool, tool uses, tokens, duration, summary, output file) is stored as JSON of
-- `dovetail_api::SubagentProgress`; NULL for sessions. `tool_use_id` and `output_file` exist
-- since 0002. Numbered 0008 because 0007 belongs to M11b (to-dos); renumber at merge if needed.
ALTER TABLE agents ADD COLUMN progress TEXT;
