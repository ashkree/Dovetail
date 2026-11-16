// The viewer's data (ADR 0026): the core's previews and declared variants as the view shows them,
// and the UI commands that become core commands.

import { describe, expect, it } from 'vitest';
import { coreSnapshot, pendingFor } from './fixtures';
import { translate } from './translate';
import { toView } from './view';

describe('the viewer in the view', () => {
  const core = coreSnapshot(0, 'shop');
  const view = toView(core, pendingFor('shop'));

  it('shows a declared set with the pick and the kept option as indices, under its effort', () => {
    expect(view.variants).toHaveLength(1);
    const set = view.variants[0];
    expect(set?.agent_id).toBe('header-toggle');
    expect(set?.slots.map(s => [s.id, s.level])).toEqual([
      ['theme', 'global'], ['layout', 'global'], ['header', 'component'], ['footer', 'component'], ['hero', 'content'],
    ]);
    expect(set?.choice).toEqual([0, 0, 0, 0, 0]);
    expect(set?.kept).toEqual([0, 0, 0, 0, 0]);
    expect(set?.effort_id).toBe(view.efforts.find(e => e.name === 'dark-mode')?.id);
  });

  it('follows a pick and a kept option', () => {
    const changed = {
      ...core,
      variants: core.variants.map(v => ({
        ...v,
        selection: { ...v.selection, theme: 'Midnight' },
        slots: v.slots.map(s => (s.name === 'footer' ? { ...s, kept: 'Full' } : s)),
      })),
    };
    const set = toView(changed, pendingFor('shop')).variants[0];
    expect(set?.choice[0]).toBe(1);
    expect(set?.kept[3]).toBe(1);
  });

  it('flattens each preview state', () => {
    const states = [
      ['not_started', 'not_started'], ['no_command', 'no_command'], ['starting', 'starting'],
      [{ running: { url: 'http://127.0.0.1:1/' } }, 'running'], [{ failed: { output: 'boom' } }, 'failed'],
    ] as const;
    for (const [state, flat] of states) {
      const p = toView({ ...core, previews: [{ agent: 'header-toggle', state }] }, pendingFor('shop')).previews[0];
      expect(p?.state).toBe(flat);
    }
    const running = toView({ ...core, previews: [{ agent: 'header-toggle', state: { running: { url: 'u' } } }] }, pendingFor('shop'));
    expect(running.previews[0]).toMatchObject({ url: 'u' });
  });

  it('leaves out a set with no slots', () => {
    const none = toView({ ...core, variants: [{ agent: 'header-toggle', slots: [], selection: {} }] }, pendingFor('shop'));
    expect(none.variants).toEqual([]);
  });
});

describe('the viewer commands', () => {
  const core = coreSnapshot(0, 'shop');
  it('become the core\'s', () => {
    expect(translate({ type: 'start_preview', agent_id: 'a' }, core).core).toEqual([{ start_preview: { agent: 'a' } }]);
    expect(translate({ type: 'restart_preview', agent_id: 'a' }, core).core).toEqual([{ restart_preview: { agent: 'a' } }]);
    expect(translate({ type: 'select_variant', agent_id: 'a', slot: 'theme', option: 'dark' }, core).core)
      .toEqual([{ select_variant: { agent: 'a', slot: 'theme', option: 'dark' } }]);
    expect(translate({ type: 'keep_variants', agent_id: 'a' }, core).core).toEqual([{ keep_variants: { agent: 'a' } }]);
    expect(translate({ type: 'open_external', url: 'http://127.0.0.1:9/' }, core).core)
      .toEqual([{ open_external: { url: 'http://127.0.0.1:9/' } }]);
  });
});
