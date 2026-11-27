-- Task #27 (ADR 0009, 2026-11-09): the Claude Code config directory (profile) a session ran
-- under. A session is resumed only under the same one; NULL means the default home of the time
-- (`$CLAUDE_CONFIG_DIR` or `~/.claude`), which is what every session before profiles ran under.
-- Subagents copy their parent's.
ALTER TABLE agents ADD COLUMN claude_home TEXT;
