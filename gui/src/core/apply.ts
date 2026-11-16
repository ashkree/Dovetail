// The reducer: the only way the client's copy of the core's Snapshot changes. Pure; a port of
// crates/dovetail-api/src/apply.rs, held to it by the shared cases in
// crates/dovetail-api/tests/fixtures/apply/ (apply.test.ts runs them here, the Rust test there).
// Only the event with seq == snapshot.seq + 1 applies; a later one is a gap (the client asks for
// a fresh snapshot), an earlier one is stale (ignored).

import type * as W from './generated';
import { variant } from './wire';

/** Most activity items, agent messages and conversation entries a client keeps (model.rs). */
export const ACTIVITY_TAIL = 500;
export const MESSAGE_TAIL = 500;
export const CONVERSATION_KEEP = 2000;

export type ApplyResult =
  | { kind: 'applied'; snapshot: W.Snapshot }
  | { kind: 'gap'; expected: number; got: number }
  | { kind: 'stale' };

export function apply(state: W.Snapshot, seq: number, event: W.Event): ApplyResult {
  const expected = state.seq + 1;
  if (seq < expected) return { kind: 'stale' };
  if (seq > expected) return { kind: 'gap', expected, got: seq };
  return { kind: 'applied', snapshot: { ...fold(state, event), seq } };
}

/** Every event's tag, from the generated union: a new event makes the switch below incomplete. */
type Tag<E> = E extends string ? E : E extends object ? keyof E : never;
type EventTag = Tag<W.Event>;

function fold(s: W.Snapshot, event: W.Event): W.Snapshot {
  const [tag, e] = variant(event) as [EventTag, unknown];
  switch (tag) {
    case 'task_upserted': {
      const task = e as W.Task;
      return { ...s, tasks: upsert(s.tasks, task, t => t.id) };
    }
    case 'task_removed':
      return { ...s, tasks: s.tasks.filter(t => t.id !== e) };
    case 'effort_upserted': {
      const effort = e as W.Effort;
      return { ...s, efforts: upsert(s.efforts, effort, x => x.id) };
    }
    case 'agent_upserted': {
      const agent = e as W.Agent;
      return { ...s, agents: upsert(s.agents, agent, a => a.id) };
    }
    case 'agent_removed': {
      const id = e as W.AgentId;
      const conversations = { ...s.conversations };
      delete conversations[id];
      const commands = { ...s.commands };
      delete commands[id];
      return {
        ...s,
        agents: s.agents.filter(a => a.id !== id),
        conversations,
        worktrees: s.worktrees.filter(w => w.agent !== id),
        commands,
      };
    }
    case 'worktree_upserted': {
      const info = e as W.WorktreeInfo;
      return { ...s, worktrees: upsert(s.worktrees, info, w => w.agent) };
    }
    case 'todos_replaced': {
      const { list, items } = e as { list: W.TodoListId; items: W.TodoItem[] };
      return { ...s, todos: [...s.todos.filter(t => t.list !== list), ...items] };
    }
    case 'inbox_upserted': {
      const item = e as W.InboxItem;
      return { ...s, inbox: upsert(s.inbox, item, i => i.id) };
    }
    case 'inbox_resolved': {
      const { id } = e as { id: W.InboxId };
      return { ...s, inbox: s.inbox.filter(i => i.id !== id) };
    }
    case 'activity_added':
      return { ...s, activity: bounded(s.activity, e as W.ActivityItem, ACTIVITY_TAIL) };
    case 'message_added':
      return { ...s, messages: bounded(s.messages, e as W.AgentMessage, MESSAGE_TAIL) };
    case 'conversation_appended': {
      const { agent, entry } = e as { agent: W.AgentId; entry: W.ConversationEntry };
      const tail = s.conversations[agent] ?? { entries: [], has_more: false };
      const at = tail.entries.findIndex(x => x.id === entry.id);
      const next = at >= 0
        ? { ...tail, entries: tail.entries.map((x, i) => (i === at ? entry : x)) }
        : appendEntry(tail, entry);
      return { ...s, conversations: { ...s.conversations, [agent]: next } };
    }
    case 'assistant_delta': {
      const { agent, entry, text } = e as { agent: W.AgentId; entry: W.EntryId; text: string };
      const tail = s.conversations[agent] ?? { entries: [], has_more: false };
      const at = tail.entries.findIndex(x => x.id === entry);
      // The core always announces an entry before streaming into it, but a client that joined
      // mid-stream keeps the text rather than dropping it.
      const next = at >= 0
        ? { ...tail, entries: tail.entries.map((x, i) => (i === at ? { ...x, text: x.text + text } : x)) }
        : appendEntry(tail, { id: entry, at_ms: s.now_ms, kind: 'assistant', text });
      return { ...s, conversations: { ...s.conversations, [agent]: next } };
    }
    case 'conversation_page': {
      const { agent, entries, has_more } =
        e as { agent: W.AgentId; entries: W.ConversationEntry[]; has_more: boolean };
      const tail = s.conversations[agent] ?? { entries: [], has_more: false };
      const older = entries.filter(x => !tail.entries.some(t => t.id === x.id));
      return {
        ...s,
        conversations: { ...s.conversations, [agent]: { entries: [...older, ...tail.entries], has_more } },
      };
    }
    case 'decision_added': {
      const decision = e as W.Decision;
      return { ...s, decisions: upsert(s.decisions, decision, d => d.id) };
    }
    case 'map_upserted': {
      // Most recently drawn first: a redrawn map moves to the front.
      const map = e as W.MapSummary;
      return { ...s, maps: [map, ...s.maps.filter(m => m.id !== map.id)] };
    }
    case 'research_upserted': {
      const summary = e as W.ResearchSummary;
      return { ...s, research: upsert(s.research, summary, r => r.task) };
    }
    case 'tangent_upserted': {
      const tangent = e as W.TangentSummary;
      return { ...s, tangents: upsert(s.tangents, tangent, t => t.task.id) };
    }
    case 'tangent_removed':
      return { ...s, tangents: s.tangents.filter(t => t.task.id !== e) };
    case 'totals_changed':
      return { ...s, totals: e as W.Totals };
    case 'usage_changed':
      return { ...s, usage: e as W.UsageLimits };
    case 'warning_set': {
      const warning = e as W.Warning;
      return { ...s, warnings: upsert(s.warnings, warning, w => w.id) };
    }
    case 'warning_cleared':
      return { ...s, warnings: s.warnings.filter(w => w.id !== e) };
    case 'commands_changed': {
      const { agent, commands } = e as { agent: W.AgentId; commands: W.SlashCommand[] };
      return { ...s, commands: { ...s.commands, [agent]: commands } };
    }
    case 'models_changed':
      return { ...s, models: e as W.ModelOption[] };
    case 'journal_changed':
      return { ...s, journal: e as W.JournalState };
    case 'upgrade_building':
      return { ...s, upgrade_building: e as boolean };
    // Nothing to store: the UI acts on these as they pass through (PASSED_THROUGH in
    // crates/dovetail-api/tests/apply_cases.rs).
    case 'browsed_loaded':
    case 'clipboard_pasted':
    case 'command_failed':
    case 'doc_loaded':
    case 'docs_listed':
    case 'lend_terminal':
    case 'map_loaded':
    case 'opening_project':
    case 'paths_listed':
    case 'project_browsed':
    case 'recipe_started':
    case 'research_loaded':
    case 'tangent_loaded':
    case 'terminal_returned':
    case 'upgrade_restarting':
      return s;
    default: {
      // A compile error here names an event this reducer does not handle yet.
      const unhandled: never = tag;
      void unhandled;
      return s;
    }
  }
}

/** Replaces the item with the same key, or appends it. */
function upsert<T, K>(items: T[], item: T, key: (t: T) => K): T[] {
  const k = key(item);
  const at = items.findIndex(x => key(x) === k);
  return at >= 0 ? items.map((x, i) => (i === at ? item : x)) : [...items, item];
}

/** Appends, dropping the oldest items beyond `cap`. */
function bounded<T>(items: T[], item: T, cap: number): T[] {
  const next = [...items, item];
  return next.length > cap ? next.slice(next.length - cap) : next;
}

/** Appends to a conversation, dropping its oldest entries beyond CONVERSATION_KEEP. */
function appendEntry(tail: W.ConversationTail, entry: W.ConversationEntry): W.ConversationTail {
  const entries = bounded(tail.entries, entry, CONVERSATION_KEEP);
  return { entries, has_more: tail.has_more || entries.length === tail.entries.length };
}
