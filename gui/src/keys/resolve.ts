// Key event → token → sequence → Action. Pure, so it is unit-tested (resolve.test.ts).
// Scope bindings win over global ones; Insert mode is its own scope and passes text through.

import type { Action } from './actions';
import { HOME_SKIPS_GLOBAL, KEYMAP, LEADER, type Binding, type LeaderGroup, type LeaderNode } from './keymap';

/** The parts of a KeyboardEvent the resolver reads. */
export interface KeyLike { key: string; code?: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }

const ARROWS: Record<string, string> = { ArrowDown: 'j', ArrowUp: 'k', ArrowLeft: 'h', ArrowRight: 'l' };

/** The token a key press names, or null for keys the keymap never binds (bare modifiers, F-keys).
 *  Arrows are h j k l in Normal mode; while typing they stay arrows (`<Down>`) so text fields keep them. */
export function tokenOf(e: KeyLike, insert: boolean): string | null {
  if (e.metaKey) return null;
  // WebKitGTK (the Tauri window on Linux) reports Shift-Tab as key `Unidentified`, code `Tab`;
  // dropped, the browser moved focus into the chat bar instead.
  let k = e.key === 'Unidentified' && e.code === 'Tab' ? 'Tab' : e.key;
  if (k === ' ') k = '<Space>';
  else if (k === 'Enter') k = e.altKey ? '<A-CR>' : '<CR>';
  else if (k === 'Escape') k = '<Esc>';
  else if (k === 'Tab') k = e.shiftKey ? '<S-Tab>' : '<Tab>';
  else if (ARROWS[k]) k = insert ? `<${k.slice(5)}>` : ARROWS[k] ?? k;
  else if (k.length > 1) return null;
  if (e.ctrlKey && k.length === 1) k = 'C-' + k.toLowerCase();
  return k;
}

/** The bindings that apply in a scope: its own first, then the global ones it does not shadow. */
export function bindingsFor(scope: string): Binding[] {
  const local = KEYMAP[scope] ?? [];
  const global = KEYMAP.global ?? [];
  if (scope === 'insert') return local;
  if (scope === 'home') return local.concat(global.filter(b => !HOME_SKIPS_GLOBAL.includes(b[0])));
  const seen = new Set(local.map(b => b[0]));
  return local.concat(global.filter(b => !seen.has(b[0])));
}

/** One level of the leader tree: the root, or a group. */
export interface LeaderLevel { name: string; keys: Record<string, LeaderNode> }

export function leaderNode(path: string[]): LeaderLevel | null {
  let level: LeaderLevel = { name: '', keys: LEADER };
  for (const k of path) {
    const next: LeaderNode | undefined = level.keys[k];
    if (!next || Array.isArray(next)) return null;
    level = next as LeaderGroup;
  }
  return level;
}

export interface KeyState { pending: string; leader: string[] | null }

export type Resolved =
  /** Run this action; the state after it. */
  | { kind: 'action'; action: Action; state: KeyState }
  /** More keys are needed (`g` of `gg`, `Space e` of `Space e w`). */
  | { kind: 'pending'; state: KeyState }
  /** Nothing bound; `unbound` names a leader path that went nowhere. */
  | { kind: 'none'; state: KeyState; unbound?: string }
  /** Insert mode: let the key type into the chat bar. */
  | { kind: 'text' };

const IDLE: KeyState = { pending: '', leader: null };
/** Keys that work while typing, from the global table (Ctrl-h / Ctrl-l, "always"). */
const ALWAYS = ['C-h', 'C-l'];

export function resolve(token: string, scope: string, state: KeyState): Resolved {
  if (scope === 'insert') {
    const b = (KEYMAP.insert ?? []).find(x => x[0] === token)
      ?? (ALWAYS.includes(token) ? (KEYMAP.global ?? []).find(x => x[0] === token) : undefined);
    return b ? { kind: 'action', action: b[1], state: IDLE } : { kind: 'text' };
  }

  if (state.leader) {
    if (token === '<Esc>') return { kind: 'none', state: IDLE };
    const node = leaderNode(state.leader)?.keys[token];
    if (!node) return { kind: 'none', state: IDLE, unbound: ['Space', ...state.leader, token].join(' ') };
    if (Array.isArray(node)) return { kind: 'action', action: node[0], state: IDLE };
    return { kind: 'pending', state: { pending: '', leader: [...state.leader, token] } };
  }
  if (token === '<Space>' && !state.pending && scope !== 'home') return { kind: 'pending', state: { pending: '', leader: [] } };

  const binds = bindingsFor(scope);
  const tryKey = (seq: string): Resolved | null => {
    const prefix = binds.some(b => b[0] !== seq && b[0].startsWith(seq) && b[1] !== 'none');
    if (prefix) return { kind: 'pending', state: { pending: seq, leader: null } };
    const exact = binds.find(b => b[0] === seq);
    if (exact) return { kind: 'action', action: exact[1], state: IDLE };
    return null;
  };
  const r = tryKey(state.pending + token);
  if (r) return r;
  if (state.pending) return tryKey(token) ?? { kind: 'none', state: IDLE };
  return { kind: 'none', state: IDLE };
}

/** The keys bound to an action in a scope (its own bindings, then global), in table order. */
export function keysFor(action: Action, scope = 'global'): string[] {
  return bindingsFor(scope).filter(b => b[1] === action).map(b => b[0]);
}

/** The Space-leader path to an action, as tokens after Space (`['e', 'w']`), or null. */
export function leaderPathFor(action: Action): string[] | null {
  for (const [k, v] of Object.entries(LEADER)) {
    if (Array.isArray(v)) { if (v[0] === action) return [k]; continue; }
    for (const [k2, leaf] of Object.entries(v.keys)) if (leaf[0] === action) return [k, k2];
  }
  return null;
}

const NAMES: Record<string, string> = {
  '<CR>': 'Enter', '<A-CR>': 'Alt-Enter', '<Esc>': 'Esc', '<Tab>': 'Tab', '<S-Tab>': 'Shift-Tab', '<Space>': 'Space',
  '<Down>': '↓', '<Up>': '↑',
};
/** How a token is written for people: `C-h` → `Ctrl-h`, `<CR>` → `Enter`. */
export function keyName(token: string): string {
  return NAMES[token] ?? (token.startsWith('C-') ? 'Ctrl-' + token.slice(2) : token);
}
