# Slash commands and `@` paths in headless sessions

Run 2026-11-10 against Claude Code 2.1.322, in a scratch git repo under the session's scratchpad.

## Where the command list comes from

**Procedure.** Start `claude -p --input-format stream-json --output-format stream-json --verbose` and write only `{"type":"control_request","request_id":"r1","request":{"subtype":"initialize"}}`. No user message.

**Result.** The answer (`control_response`, `subtype: success`) carries `commands: [{name, description, argumentHint, aliases?}]` (90 entries: built-ins, user skills, plugin skills; a skill with an alias is listed twice), `agents: [{name, description}]`, `models`, `output_style` and more. A `system/commands_changed` line with the same `commands` list follows. No model call is made. `system/init` lists `slash_commands` and `skills` too, but by name only.

**Used as:** the driver sends `initialize` (request id `dt_initialize`) at start and maps both the answer and `commands_changed` to `DriverEvent::Commands`, deduplicated by name.

## Does `@path` work headless?

**Procedure.** Same flags, `--model claude-haiku-5-5`, `--disallowedTools Read Bash Grep Glob Task`; a file `secret.txt` holding `the word is pineapple-42`; one user message: "What word is in @secret.txt? Reply with only the word, or NONE if you cannot see the file."

**Result.** The reply was `pineapple-42`, with no tool call ($0.006). Claude Code expands `@path` itself in headless sessions, so Dovetail sends the text as typed and only helps type the path.
