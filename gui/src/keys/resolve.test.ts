import { describe, expect, it } from 'vitest';
import { resolve, tokenOf, type KeyState } from './resolve';

const idle: KeyState = { pending: '', leader: null };
const key = (k: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; code: string }> = {}) =>
  ({ key: k, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });

/** Feed tokens one by one, as the key listener does. */
function run(scope: string, tokens: string[]) {
  let state = idle;
  let last = resolve(tokens[0] ?? '', scope, state);
  for (const t of tokens) {
    last = resolve(t, scope, state);
    if (last.kind !== 'text') state = last.state;
  }
  return last;
}

describe('tokenOf', () => {
  it('names special keys and Ctrl chords', () => {
    expect(tokenOf(key(' '), false)).toBe('<Space>');
    expect(tokenOf(key('Enter', { altKey: true }), true)).toBe('<A-CR>');
    expect(tokenOf(key('Tab', { shiftKey: true }), false)).toBe('<S-Tab>');
    // WebKitGTK's Shift-Tab.
    expect(tokenOf(key('Unidentified', { shiftKey: true, code: 'Tab' }), false)).toBe('<S-Tab>');
    expect(tokenOf(key('h', { ctrlKey: true }), false)).toBe('C-h');
    expect(tokenOf(key('ArrowDown'), false)).toBe('j');
    expect(tokenOf(key('Shift'), false)).toBeNull();
  });
});

describe('resolve', () => {
  it('runs a single key', () => {
    expect(run('inbox', ['y'])).toMatchObject({ kind: 'action', action: 'allow' });
  });

  it('waits for and runs a two-key sequence', () => {
    expect(run('chat', ['g'])).toMatchObject({ kind: 'pending', state: { pending: 'g' } });
    expect(run('chat', ['g', 'g'])).toMatchObject({ kind: 'action', action: 'top' });
    expect(run('chat', [']', 'e'])).toMatchObject({ kind: 'action', action: 'nextEffort' });
  });

  it('walks a leader path', () => {
    expect(run('chat', ['<Space>'])).toMatchObject({ kind: 'pending', state: { leader: [] } });
    expect(run('chat', ['<Space>', 'e'])).toMatchObject({ kind: 'pending', state: { leader: ['e'] } });
    expect(run('chat', ['<Space>', 'e', 'w'])).toMatchObject({ kind: 'action', action: 'wwi' });
    expect(run('chat', ['<Space>', 'q'])).toMatchObject({ kind: 'none', unbound: 'Space q' });
  });

  it('lets scope bindings win over global ones', () => {
    expect(run('chat', ['h'])).toMatchObject({ kind: 'action', action: 'zoneLeft' });
    expect(run('canvas', ['h'])).toMatchObject({ kind: 'action', action: 'cardLeft' });
    expect(run('home', ['h'])).toMatchObject({ kind: 'action', action: 'none' });
  });

  it('passes text through in Insert mode, but keeps its own keys', () => {
    expect(run('insert', ['j'])).toEqual({ kind: 'text' });
    expect(run('insert', ['<Space>'])).toEqual({ kind: 'text' });
    expect(run('insert', ['<CR>'])).toMatchObject({ kind: 'action', action: 'send' });
    expect(run('insert', ['C-h'])).toMatchObject({ kind: 'action', action: 'zoneLeft' });
  });
});
