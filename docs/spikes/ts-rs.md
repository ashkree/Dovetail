# Spike: `ts-rs` for the GUI's TypeScript types

Gates ADR 0023, point 4. Run 2026-11-16 in `spikes/ts-rs/` (a throwaway crate, not a workspace member) with `ts-rs` 12.0.1, features `serde-json-impl` and `no-serde-warnings`.

## What was checked

One type per serde pattern `crates/dovetail-api` uses, exported with `TS::decl` and compared with what `serde_json` writes for the same value.

| Pattern in `dovetail-api` | Example | TypeScript out | Matches serde |
| --- | --- | --- | --- |
| `#[serde(transparent)]` id | `TaskId(i64)`, `AgentId(String)` | `type TaskId = bigint` / `type AgentId = string` | yes for `AgentId`; `TaskId` see below |
| unit enum, `rename_all = "snake_case"` | `TaskStatus` | `"todo" \| "in_progress"` | yes |
| enum with struct and tuple variants (externally tagged) | `Event::TaskRemoved(TaskId)`, `Event::AssistantDelta {..}`, `Event::LendTerminal` | `{ "task_removed": TaskId } \| { "assistant_delta": {..} } \| "lend_terminal"` | yes |
| internally tagged enum (`tag = "kind"`) | `SocketResponseBody` | `{ "kind": "tool_result", text: string }` | yes |
| `default` + `skip_serializing_if` | `Snapshot.profile` | `profile?: string \| null` | yes |
| `serde_json::Value` | `InboxKind::Permission.input` | `JsonValue` (exported alongside) | yes |
| `BTreeMap<AgentId, _>` | `Snapshot.conversations` | `{ [key in AgentId]: … }` | yes |
| `PathBuf` | `ProjectSummary.repo_path` | `string` | yes |
| variant `rename` | `ImageType::Png` = `"image/png"` | `"image/png"` | yes |

## Result: pass, with one setting

64-bit integers (`i64`, `u64`: every id, `seq`, every `*_ms`) export as `bigint` by default, but `JSON.parse` gives a `number`. `Config::with_large_int("number")` fixes it for every type at once. Ids and times stay well under 2^53, so `number` is exact.

No alternative was needed. `ts-rs` keeps the plain-data API crate free of Tauri (ADR 0023 rejects `tauri-specta` for that), and an optional feature keeps it out of ordinary builds.
