// Actions that failed in the first live run against the real core (docs/plan/smoke-test.md):
// each is checked against a fake core link that records what the GUI sends.

import { beforeEach, describe, expect, it } from 'vitest';
import { coreSnapshot } from '../core/fixtures';
import type * as W from '../core/generated';
import type { CoreLink } from '../core/link';
import { connect, coreStore } from '../state/store';
import { setUi, uiStore } from '../state/ui';
import { dispatch, slotIndex } from './actions';

let sent: W.Command[] = [];
let push: (msg: W.ServerMsg) => void = () => {};

/** A core whose snapshot is `snap`; what the GUI sends lands in `sent`. */
function fakeCore(snap: W.Snapshot): void {
  sent = [];
  const link: CoreLink = {
    subscribe(fn) { push = fn; fn({ snapshot: snap }); return () => {}; },
    send(cmd) { sent.push(cmd); },
  };
  connect(link);
}

describe('actions against the core', () => {
  beforeEach(() => fakeCore(coreSnapshot(0, 'shop')));

  it('sends a message with no effort in focus to Unsorted', () => {
    setUi({ screen: 'ws', effort: null, tag: null, draft: 'hello' });
    dispatch('send');
    expect(sent).toEqual([{ prompt: { agent: 'coordinator', text: 'hello', effort: 1 } }]);
  });

  it('cycles the coordinator\'s permission mode', () => {
    dispatch('permMode');
    expect(sent).toEqual([{ cycle_permission_mode: { agent: 'coordinator' } }]);
  });

  it('focuses a new effort when the core adds it', () => {
    const seq = coreStore.get().seq + 1;
    const effort: W.Effort = {
      id: 99, name: 'fresh', kind: 'feature', question_or_goal: 'g', status: 'open',
      where_was_i: { last_done: '', waits_on_you: '', next_step: '' }, criteria: [],
    };
    setUi({ effort: null, awaitEffort: 'fresh' });
    push({ event: { seq, event: { effort_upserted: effort } } });
    expect(uiStore.get().effort).toBe('99');
    expect(uiStore.get().awaitEffort).toBeNull();
  });

  it('starts a canvas with 1 2 3 only while the effort has none', () => {
    const withCanvas = coreStore.get().canvases[0];
    expect(withCanvas).toBeDefined();
    setUi({ effort: String(withCanvas!.effort) });
    dispatch('framework2');
    expect(sent).toEqual([]);
    const bare = coreStore.get().efforts.find(e => !coreStore.get().canvases.some(c => c.effort === e.id));
    setUi({ effort: String(bare!.id) });
    dispatch('framework2');
    expect(sent).toEqual([{ create_canvas: { effort: bare!.id, framework: 'options_compare', title: '' } }]);
  });
});

describe('slotIndex', () => {
  it('keeps the remembered slot inside a smaller set', () => {
    const set = { agent_id: 'a', effort_id: '1', slots: [{ id: 'x', name: 'x', level: 'content' as const, options: ['a'] }], choice: [0], kept: [0] };
    expect(slotIndex(set, 2)).toBe(0);
    expect(slotIndex(set, 0)).toBe(0);
  });
});
