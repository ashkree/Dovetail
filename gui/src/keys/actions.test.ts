// Actions that failed in the first live run against the real core (docs/plan/smoke-test.md):
// each is checked against a fake core link that records what the GUI sends.

import { beforeEach, describe, expect, it } from 'vitest';
import { coreSnapshot } from '../core/fixtures';
import type * as W from '../core/generated';
import type { CoreLink } from '../core/link';
import { connect, coreStore, snapshotStore } from '../state/store';
import { needsYou, treeRows } from '../state/select';
import { setUi, uiStore } from '../state/ui';
import { dispatch, markSeen, slotIndex, viewerAgent } from './actions';

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
    setUi({ effort: null, awaitEffort: true });
    push({ event: { seq, event: { effort_upserted: effort } } });
    expect(uiStore.get().effort).toBe('99');
    expect(uiStore.get().awaitEffort).toBe(false);
  });

  it('forgets a new effort the core refused', () => {
    setUi({ effort: null, awaitEffort: true });
    push({ event: { seq: coreStore.get().seq + 1, event: { command_failed: { message: 'taken' } } } });
    expect(uiStore.get().awaitEffort).toBe(false);
  });

  it('b 1 2 3 add a block of that layout to the focused effort', () => {
    setUi({ effort: '3' });
    dispatch('newBlock2');
    expect(sent).toEqual([{ create_block: { effort: 3, title: '', layout: 'options_compare', near: null } }]);
  });

  it('marks a selected card seen only when an agent left it unseen', () => {
    const card = coreStore.get().cards.find(c => c.block != null)!;
    markSeen(snapshotStore.get(), `Q${card.id}`);
    expect(sent).toEqual([]);
    push({ event: { seq: coreStore.get().seq + 1, event: { card_upserted: { ...card, seen: false } } } });
    markSeen(snapshotStore.get(), `Q${card.id}`);
    expect(sent).toEqual([{ see_card: { id: card.id } }]);
  });

  it('lets go of a picked slice when the effort changes', () => {
    setUi({ viewerAgent: 'login-fix' });
    dispatch('switchEffort', '2');
    expect(uiStore.get().viewerAgent).toBeNull();
  });

  it('opens the viewer on a slice picked in the tree, whatever effort has focus', () => {
    const rows = treeRows(snapshotStore.get(), {});
    const i = rows.findIndex(r => r.type === 'agent' && r.agent.kind === 'slice' && r.agent.id === 'login-fix');
    setUi({ screen: 'ws', zone: 'tree', effort: '1', folded: {}, treeSel: i, viewerAgent: null });
    dispatch('openRow');
    expect(uiStore.get()).toMatchObject({ viewerAgent: 'login-fix', main: 'viewer', effort: '1' });
    expect(viewerAgent(snapshotStore.get(), uiStore.get())?.id).toBe('login-fix');
  });

  it('cycles the mode of the agent under the tree cursor', () => {
    const rows = treeRows(snapshotStore.get(), {});
    const i = rows.findIndex(r => r.type === 'agent' && r.agent.id === 'run-e2e');
    setUi({ screen: 'ws', zone: 'tree', mode: 'normal', folded: {}, treeSel: i });
    dispatch('permMode');
    // A subagent shares its parent's session.
    expect(sent).toEqual([{ cycle_permission_mode: { agent: 'login-fix' } }]);
  });

  it('selects a project the user just created on Home', () => {
    setUi({ screen: 'home', homeSel: 0, awaitProject: true });
    const p = { ...coreStore.get().projects[0]!, id: 'fresh', name: 'Fresh' };
    push({ event: { seq: coreStore.get().seq + 1, event: { project_upserted: p } } });
    expect(uiStore.get().homeSel).toBe(snapshotStore.get().projects.findIndex(x => x.id === 'fresh'));
    expect(uiStore.get().awaitProject).toBe(false);
  });

  it('counts only items that wait on the user', () => {
    const s = snapshotStore.get();
    expect(needsYou(s)).toBe(s.inbox.filter(i => i.kind !== 'finished').length);
    expect(s.inbox.findIndex(i => i.kind === 'finished')).toBeGreaterThanOrEqual(needsYou(s));
  });
});

describe('slotIndex', () => {
  it('keeps the remembered slot inside a smaller set', () => {
    const set = { agent_id: 'a', effort_id: '1', slots: [{ id: 'x', name: 'x', level: 'content' as const, options: ['a'] }], choice: [0], kept: [0] };
    expect(slotIndex(set, 2)).toBe(0);
    expect(slotIndex(set, 0)).toBe(0);
  });
});
