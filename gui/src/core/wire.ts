// Helpers for the core's wire format (the generated types in ./generated, from dovetail-api).
// serde writes an enum value as `"name"` (no data) or `{ "name": data }`; these read it.

import type * as W from './generated';

/** The variant name and its data (undefined for a variant with none). */
export function variant<T extends string | object>(value: T): [string, unknown] {
  if (typeof value === 'string') return [value, undefined];
  const entries = Object.entries(value);
  if (entries.length !== 1) throw new Error(`not an enum value: ${JSON.stringify(value)}`);
  return entries[0] as [string, unknown];
}

/** The id of the coordinator's agent (dovetail_core::COORDINATOR). */
export const COORDINATOR: W.AgentId = 'coordinator';

/** The effort every project starts with (dovetail_api::UNSORTED_EFFORT). */
export const UNSORTED_EFFORT: W.EffortId = 1;

/** A snapshot with nothing in it: what the client holds before the core's first message. */
export function emptyCoreSnapshot(): W.Snapshot {
  return {
    seq: 0, now_ms: 0, project: null, projects: [], efforts: [], tasks: [], agents: [], todos: [],
    inbox: [], activity: [], messages: [], conversations: {},
    totals: { memory_mb: 0, needs_you: 0, cost_usd: 0 }, warnings: [], decisions: [],
    usage: { five_hour: null, seven_day: null }, worktrees: [], maps: [], research: [], commands: {},
    upgrade_building: false, tangents: [], models: [],
  };
}
