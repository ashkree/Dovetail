// The reducer: the only way the client's snapshot changes. Pure; mirrors
// crates/dovetail-api/src/apply.rs. Only the event with seq == snapshot.seq + 1 applies; a later
// one is a gap (the client asks for a fresh snapshot), an earlier one is stale (ignored).

import type { Snapshot } from './model';
import type { Event } from './protocol';

export type ApplyResult =
  | { kind: 'applied'; snapshot: Snapshot }
  | { kind: 'gap'; expected: number; got: number }
  | { kind: 'stale' };

export function apply(state: Snapshot, seq: number, event: Event): ApplyResult {
  const expected = state.seq + 1;
  if (seq < expected) return { kind: 'stale' };
  if (seq > expected) return { kind: 'gap', expected, got: seq };
  return { kind: 'applied', snapshot: { ...fold(state, event), seq } };
}

function fold(s: Snapshot, e: Event): Snapshot {
  switch (e.type) {
    case 'chat_entry_added':
      return { ...s, chat: [...s.chat, e.entry] };
    case 'inbox_resolved':
      return { ...s, inbox: s.inbox.filter(i => i.id !== e.id) };
    case 'criterion_changed':
      return {
        ...s,
        efforts: s.efforts.map(ef => ef.id !== e.effort_id ? ef : {
          ...ef, criteria: ef.criteria.map(c => c.id === e.criterion_id ? { ...c, state: e.state } : c),
        }),
      };
    case 'card_added':
      return { ...s, cards: [...s.cards.filter(c => c.id !== e.card.id), e.card] };
    case 'card_link_added':
      return { ...s, card_links: [...s.card_links, e.link] };
    case 'incoming_removed':
      return { ...s, incoming: s.incoming.filter(c => c.id !== e.id) };
    case 'card_moved':
      return { ...s, cards: s.cards.map(c => c.id === e.id ? { ...c, x: e.x, y: e.y } : c) };
    case 'card_deleted':
      return {
        ...s,
        cards: s.cards.filter(c => c.id !== e.id),
        card_links: s.card_links.filter(l => l.from !== e.id && l.to !== e.id),
      };
    case 'task_added':
      return { ...s, tasks: [...s.tasks, e.task] };
    case 'effort_renamed':
      return { ...s, efforts: s.efforts.map(ef => ef.id === e.id ? { ...ef, name: e.name } : ef) };
    case 'effort_added':
      return { ...s, efforts: [...s.efforts, e.effort] };
    case 'variants_kept':
      return { ...s, variants: s.variants.map(v => v.agent_id === e.agent_id ? { ...v, kept: e.kept } : v) };
    case 'status_changed':
      return { ...s, status: e.status };
  }
}
