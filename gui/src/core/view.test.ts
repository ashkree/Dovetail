// The view made from a core Snapshot: the mock's Shop goes through the core's shapes and back,
// and must come out as the screens show it (fixtures.ts, in the view's terms).

import { describe, expect, it } from 'vitest';
import { coreSnapshot, pendingFor } from './fixtures';
import { emptyPending } from './pending';
import { modelName, openedText, toView } from './view';

describe('toView', () => {
  const view = toView(coreSnapshot(0, 'shop'), pendingFor('shop'));

  it('keeps the efforts in order, hides an empty Unsorted, and maps the card', () => {
    expect(view.efforts.map(e => e.name)).toEqual(['payment-providers', 'login-bug', 'dark-mode', 'search-index', 'coupon-ab']);
    const login = view.efforts[1];
    expect(login?.where_was_i.last).toMatch(/^login-fix set/);
    expect(login?.criteria.map(c => c.state)).toEqual(['met', 'claimed', 'open']);
    expect(login?.colour).toBe('coral');
  });

  it('shows a claimed criterion as an inbox row first, then the core items', () => {
    expect(view.inbox.map(i => i.kind)).toEqual(['criterion', 'permission', 'question', 'finished']);
    expect(view.inbox[0]).toMatchObject({ id: 'c5', title: 'Criterion · Fix passes in Safari and Chrome' });
    expect(view.inbox[1]?.title).toBe('Bash · npm run e2e -- --browser safari');
    // Each item belongs to its agent's task's effort.
    expect(view.inbox[1]?.effort_id).toBe(view.efforts[1]?.id);
  });

  it("reads the coordinator's conversation as the chat, reports by agent", () => {
    expect(view.chat).toHaveLength(9);
    const report = view.chat.find(m => m.report);
    expect(report).toMatchObject({ who: 'research-1', at: '09:20' });
    expect(report?.text).toMatch(/^SCA comparison written/);
    expect(view.chat[0]).toMatchObject({ who: 'you', effort_id: view.efforts[1]?.id });
  });

  it('puts agents under their tasks and subagents under their slice', () => {
    const sub = view.agents.find(a => a.id === 'run-e2e');
    const slice = view.agents.find(a => a.id === 'login-fix');
    expect(sub).toMatchObject({ kind: 'subagent', parent_id: 'login-fix', task_id: slice?.task_id });
    expect(slice?.branch).toBe('fix/login-safari');
    expect(view.agents.some(a => a.id === 'coordinator')).toBe(false);
  });

  it('fills the bottom bar from the coordinator and the usage', () => {
    expect(view.status).toMatchObject({ model: 'Opus 5.5', effort_level: 'Medium', permission_mode: 'Auto', context_fill: 0.62, busy: true });
    expect(view.status.usage).toEqual({ five_hour: 0.34, week: 0.18 });
  });

  it("gives Home each project's repos, account and counts from the core", () => {
    const home = toView(coreSnapshot(0, null), emptyPending());
    expect(home.open_project).toBeNull();
    expect(home.projects.find(p => p.id === 'thesis')?.repos).toEqual([]);
    expect(home.projects.find(p => p.id === 'shop')).toMatchObject({
      repos: ['shop-api', 'shop-web'], account: 'work', efforts: 5, running: 5, needs_you: 4,
    });
    expect(home.projects.find(p => p.id === 'shop')?.opened).toMatch(/^today \d\d:\d\d$/);
    expect(home.projects.find(p => p.id === 'dovetail')?.opened).toBe('yesterday');
  });

  it('says when a project was last opened', () => {
    const now = new Date(2026, 10, 16, 15, 0).getTime();
    expect(openedText(new Date(2026, 10, 16, 9, 5).getTime(), now)).toBe('today 09:05');
    expect(openedText(new Date(2026, 10, 15, 23, 0).getTime(), now)).toBe('yesterday');
    expect(openedText(new Date(2026, 10, 2, 8, 0).getTime(), now)).toBe('2 Nov');
    expect(openedText(null, now)).toBe('');
  });

  it('names models as people say them', () => {
    expect(modelName('claude-opus-5-5')).toBe('Opus 5.5');
    expect(modelName('opus')).toBe('opus');
  });
});
