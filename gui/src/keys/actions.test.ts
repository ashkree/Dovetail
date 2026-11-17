// Actions that failed in the first live run against the real core (docs/plan/smoke-test.md):
// each is checked against a fake core link that records what the GUI sends.

import { beforeEach, describe, expect, it } from 'vitest';
import { coreSnapshot } from '../core/fixtures';
import type * as W from '../core/generated';
import type { CoreLink } from '../core/link';
import { taskOrder } from '../core/wire';
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

  it('types to the agent whose conversation is open, tagged with its effort', () => {
    const agent = snapshotStore.get().agents.find(a => a.id === 'login-fix')!;
    dispatch('showAgent', 'login-fix');
    expect(uiStore.get()).toMatchObject({ main: 'agent', agent: 'login-fix', zone: 'main' });
    dispatch('insert');
    expect(uiStore.get()).toMatchObject({ mode: 'insert', zone: 'main' });
    setUi({ agentDraft: 'status?', draft: 'for the coordinator' });
    dispatch('send');
    expect(sent).toEqual([{ prompt: { agent: 'login-fix', text: 'status?', effort: Number(agent.effort_id) } }]);
    expect(uiStore.get()).toMatchObject({ agentDraft: '', draft: 'for the coordinator' });
  });

  it('answers the open agent\'s permission request from its view', () => {
    dispatch('showAgent', 'login-fix');
    dispatch('allow');
    const item = coreStore.get().inbox.find(i => i.agent === 'login-fix' && 'permission' in (i.kind as object))!;
    expect(sent).toEqual([{ answer_permission: { item: item.id, answer: 'allow' } }]);
  });

  it('interrupts the agent in focus, a subagent through its slice, else the coordinator', () => {
    dispatch('showAgent', 'run-e2e');
    dispatch('interrupt');
    setUi({ zone: 'chat', main: null });
    dispatch('interrupt');
    expect(sent).toEqual([{ interrupt: { agent: 'login-fix' } }, { interrupt: { agent: 'coordinator' } }]);
  });

  it('archives a task only after the confirm', () => {
    const rows = treeRows(snapshotStore.get(), {});
    const i = rows.findIndex(r => r.type === 'task');
    const task = rows[i]!.type === 'task' ? (rows[i] as { task: { id: string } }).task : null;
    setUi({ screen: 'ws', zone: 'tree', mode: 'normal', folded: {}, treeSel: i });
    dispatch('archive');
    expect(sent).toEqual([]);
    const m = uiStore.get().modal;
    if (m?.type !== 'picker') throw new Error('no confirm');
    m.items[1]!.go();
    expect(sent).toEqual([{ archive_task: { id: Number(task!.id) } }]);
  });

  it('moves a task past one task of its own effort', () => {
    const core = coreStore.get();
    const effort = core.tasks.find(t => core.tasks.filter(x => x.effort === t.effort).length > 1)!.effort;
    const mine = core.tasks.filter(t => t.effort === effort && t.status !== 'archived').sort(taskOrder);
    const rows = treeRows(snapshotStore.get(), {});
    const i = rows.findIndex(r => r.type === 'task' && r.task.id === String(mine[0]!.id));
    setUi({ screen: 'ws', zone: 'tree', mode: 'normal', folded: {}, treeSel: i });
    dispatch('moveUp');
    expect(sent).toEqual([]);
    dispatch('moveDown');
    const peers = core.tasks.filter(t => t.status !== 'archived' && t.bucket === mine[0]!.bucket).sort(taskOrder);
    const delta = peers.findIndex(t => t.id === mine[1]!.id) - peers.findIndex(t => t.id === mine[0]!.id);
    expect(sent).toEqual([{ reorder_task: { id: mine[0]!.id, delta } }]);
  });

  it('keeps the tree cursor on a moved task once the core reorders', () => {
    const core = coreStore.get();
    const [a, b] = core.tasks.filter(t => t.effort === core.tasks[0]!.effort && t.status !== 'archived')
      .sort((x, y) => x.order - y.order || x.id - y.id);
    const at = () => treeRows(snapshotStore.get(), {}).findIndex(r => r.type === 'task' && r.task.id === String(a!.id));
    const before = at();
    setUi({ screen: 'ws', zone: 'tree', mode: 'normal', folded: {}, treeSel: before });
    dispatch('moveDown');
    const seq = coreStore.get().seq;
    push({ event: { seq: seq + 1, event: { task_upserted: { ...b!, order: 0 } } } });
    push({ event: { seq: seq + 2, event: { task_upserted: { ...a!, order: 1 } } } });
    expect(at()).toBeGreaterThan(before);
    expect(uiStore.get().treeSel).toBe(at());
  });

  it('says why a task cannot pass one in another bucket', () => {
    const core = coreStore.get();
    const [a, b] = core.tasks.filter(t => t.effort === core.tasks[0]!.effort && t.status !== 'archived').sort(taskOrder);
    push({ event: { seq: core.seq + 1, event: { task_upserted: { ...a!, bucket: 'now' } } } });
    const rows = treeRows(snapshotStore.get(), {});
    setUi({ screen: 'ws', zone: 'tree', mode: 'normal', folded: {}, treeSel: rows.findIndex(r => r.type === 'task' && r.task.id === String(b!.id)) });
    dispatch('moveUp');
    expect(sent).toEqual([]);
    expect(uiStore.get().toast?.text).toContain('within a bucket');
  });

  it('counts an agent error as needing the user (ADR 0028)', () => {
    const before = needsYou(snapshotStore.get());
    const item: W.InboxItem = { id: 900, agent: 'coordinator', at_ms: 0, kind: { error: { text: 'session ended unexpectedly' } } };
    push({ event: { seq: coreStore.get().seq + 1, event: { inbox_upserted: item } } });
    expect(snapshotStore.get().inbox.find(i => i.id === 'i900')?.kind).toBe('error');
    expect(needsYou(snapshotStore.get())).toBe(before + 1);
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
