import { describe, expect, it } from 'vitest';
import { apply } from './apply';
import { emptySnapshot } from './model';
import type { Event } from './protocol';

const entry: Event = { type: 'chat_entry_added', entry: { id: 'm1', who: 'you', report: false, at: '10:00', effort_id: 'e1', text: 'hi' } };

describe('apply', () => {
  it('applies the next event and advances seq', () => {
    const s = { ...emptySnapshot(), seq: 4 };
    const r = apply(s, 5, entry);
    expect(r.kind).toBe('applied');
    if (r.kind !== 'applied') return;
    expect(r.snapshot.seq).toBe(5);
    expect(r.snapshot.chat.map(m => m.id)).toEqual(['m1']);
    expect(s.chat).toEqual([]); // pure: the input is untouched
  });

  it('rejects a gap', () => {
    const r = apply({ ...emptySnapshot(), seq: 4 }, 7, entry);
    expect(r).toEqual({ kind: 'gap', expected: 5, got: 7 });
  });

  it('ignores a stale event', () => {
    expect(apply({ ...emptySnapshot(), seq: 4 }, 4, entry)).toEqual({ kind: 'stale' });
    expect(apply({ ...emptySnapshot(), seq: 4 }, 2, entry)).toEqual({ kind: 'stale' });
  });

  it('deletes a card with its links', () => {
    const s = {
      ...emptySnapshot(),
      cards: [{ id: 'A', canvas_id: 'c', kind: 'idea' as const, x: 0, y: 0, title: '', body: '' }],
      card_links: [{ from: 'A', to: 'B' }, { from: 'B', to: 'C' }],
    };
    const r = apply(s, 1, { type: 'card_deleted', id: 'A' });
    if (r.kind !== 'applied') throw new Error(r.kind);
    expect(r.snapshot.cards).toEqual([]);
    expect(r.snapshot.card_links).toEqual([{ from: 'B', to: 'C' }]);
  });

  it('ticks a criterion', () => {
    const s = {
      ...emptySnapshot(),
      efforts: [{
        id: 'e', name: 'x', kind: 'fix' as const, question_or_goal: '', running: true, colour: 'blue' as const,
        where_was_i: { last: '', waits: '', next: '' }, criteria: [{ id: 'c', text: '', state: 'claimed' as const }],
      }],
    };
    const r = apply(s, 1, { type: 'criterion_changed', effort_id: 'e', criterion_id: 'c', state: 'met' });
    if (r.kind !== 'applied') throw new Error(r.kind);
    expect(r.snapshot.efforts[0]?.criteria[0]?.state).toBe('met');
  });
});
