import { describe, expect, it } from 'vitest';
import { apply, applyBatch, type SeqEvent } from './apply';
import type * as W from './generated';
import { COORDINATOR, emptyCoreSnapshot } from './wire';

const entry = (id: number): W.ConversationEntry => ({ id, at_ms: 1, kind: 'assistant', text: '' });
const appended = (seq: number, id: number): SeqEvent => ({ seq, event: { conversation_appended: { agent: COORDINATOR, entry: entry(id) } } });
const delta = (seq: number, id: number, text: string): SeqEvent => ({ seq, event: { assistant_delta: { agent: COORDINATOR, entry: id, text } } });

/** What applying the events one at a time gives: the batch must equal it. */
function oneByOne(events: SeqEvent[]): W.Snapshot {
  let s = emptyCoreSnapshot();
  for (const { seq, event } of events) {
    const r = apply(s, seq, event);
    if (r.kind === 'applied') s = r.snapshot;
  }
  return s;
}

describe('applyBatch', () => {
  it('gives the same snapshot as applying one by one, joining runs of deltas', () => {
    const events = [appended(1, 7), delta(2, 7, 'a '), delta(3, 7, 'b '), delta(4, 7, 'c'), appended(5, 8), delta(6, 8, 'x'), delta(7, 7, '!')];
    const { snapshot, gap } = applyBatch(emptyCoreSnapshot(), events);
    expect(gap).toBeNull();
    expect(snapshot).toEqual(oneByOne(events));
    expect(snapshot.seq).toBe(7);
    expect(snapshot.conversations[COORDINATOR]?.entries.map(e => e.text)).toEqual(['a b c!', 'x']);
  });

  it('stops at a gap and keeps what came before it', () => {
    const { snapshot, gap } = applyBatch(emptyCoreSnapshot(), [appended(1, 7), delta(2, 7, 'a'), delta(4, 7, 'late')]);
    expect(gap).toEqual({ expected: 3, got: 4 });
    expect(snapshot.seq).toBe(2);
    expect(snapshot.conversations[COORDINATOR]?.entries[0]?.text).toBe('a');
  });

  it('does not join deltas that are not consecutive', () => {
    const { gap } = applyBatch(emptyCoreSnapshot(), [appended(1, 7), delta(2, 7, 'a'), delta(4, 7, 'b')]);
    expect(gap?.got).toBe(4);
  });

  it('skips stale events', () => {
    const { snapshot, gap } = applyBatch(emptyCoreSnapshot(), [appended(1, 7), delta(1, 7, 'again'), delta(2, 7, 'a')]);
    expect(gap).toBeNull();
    expect(snapshot.conversations[COORDINATOR]?.entries[0]?.text).toBe('a');
  });
});
