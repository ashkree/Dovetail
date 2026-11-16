// The `?` keyboard map: a drawn keyboard for the scope in focus, layer by layer (plain, Shift,
// Ctrl, Space leader). Drawn from the keymap table, so it cannot drift from what the keys do.

import type { ReactNode } from 'react';
import { KEYMAP, LEADER, OVERLAY_KEYS, SCOPES, type LeaderGroup } from '../../keys/keymap';
import { bindingsFor, keyName, keysFor, leaderNode } from '../../keys/resolve';
import type { Action } from '../../keys/actions';
import { setUi, useUi, type Overlay, type OverlayLayer } from '../../state/ui';

/** A physical key: [token, cap, shifted token, width class]. The board's layout, not bindings. */
type KeyDef = [base: string | null, cap?: string, shifted?: string | null, w?: string];
const KB_ROWS: KeyDef[][] = [
  [['<Esc>', 'Esc', null, 'w15'], ['1', '1', '!'], ['2', '2', '@'], ['3', '3', '#'], ['4', '4', '$'], ['5', '5', '%'], ['6', '6', '^'], ['7', '7', '&'], ['8', '8', '*'], ['9', '9', '('], ['0', '0', ')'], ['-', '-', '_'], ['=', '=', '+'], [null, '⌫', null, 'w15']],
  [['<Tab>', 'Tab', '<S-Tab>', 'w15'], ['q'], ['w'], ['e'], ['r'], ['t'], ['y'], ['u'], ['i'], ['o'], ['p'], ['[', '[', '{'], [']', ']', '}'], ['\\', '\\', '|']],
  [[null, 'Caps', null, 'w2'], ['a'], ['s'], ['d'], ['f'], ['g'], ['h'], ['j'], ['k'], ['l'], [';', ';', ':'], ["'", "'", '"'], ['<CR>', 'Enter', null, 'w2']],
  [[null, 'Shift', null, 'w25'], ['z'], ['x'], ['c'], ['v'], ['b'], ['n'], ['m'], [',', ',', '<'], ['.', '.', '>'], ['/', '/', '?'], [null, 'Shift', null, 'w25']],
  [['<Space>', 'Space', null, 'space']],
];

interface KeyInfo { w: string; cap: string; label: ReactNode; cls: string; group?: string }

function keyInfo(def: KeyDef, ov: Overlay): KeyInfo {
  const [base, capIn, shiftedIn, wIn] = def;
  const cap = capIn ?? base ?? '';
  const w = wIn ?? '';
  const shifted = base && /^[a-z]$/.test(base) ? base.toUpperCase() : shiftedIn ?? null;
  const { scope, layer } = ov;
  let token: string | null = null;
  let shownCap = cap;
  if (layer === 'plain') token = base;
  else if (layer === 'shift') { token = shifted; if (shifted && shifted.length === 1) shownCap = shifted; }
  else if (layer === 'ctrl') token = base && /^[a-z]$/.test(base) ? 'C-' + base : null;
  else {
    if (base === '<Esc>') return { w, cap: shownCap, label: ov.leader.length ? 'up a level' : 'cancel', cls: 'c-mode' };
    const node = base ? leaderNode(ov.leader)?.keys[base] : undefined;
    if (!node || !base) return { w, cap: shownCap, label: '', cls: '' };
    if (Array.isArray(node)) return { w, cap: shownCap, label: node[1], cls: 'c-view' };
    return { w, cap: shownCap, label: (node as LeaderGroup).name + ' ▸', cls: 'c-group', group: base };
  }
  if (!token) return { w, cap: shownCap, label: '', cls: '' };
  if (layer === 'plain' && token === '<Space>' && scope !== 'insert') return { w, cap: shownCap, label: 'leader ▸ (see Space layer)', cls: 'c-group' };
  const binds = bindingsFor(scope);
  const local = new Set((KEYMAP[scope] ?? []).map(b => b[0]));
  const exact = binds.find(b => b[0] === token && b[2]);
  const pre = binds.filter(b => b[0].length > token.length && b[0].startsWith(token) && !token.startsWith('<') && b[2]);
  if (exact) {
    const more = pre.length ? ' · ' + pre.map(p => `${p[0]} ${p[2]}`).join(', ') : '';
    return { w, cap: shownCap, label: exact[2] + more, cls: `c-${exact[3]}${local.has(token) ? ' c-local' : ''}` };
  }
  if (pre.length) {
    return { w, cap: shownCap, cls: 'c-pre', label: pre.map((p, i) => <span key={p[0]}>{i > 0 && <br />}<b style={{ color: 'var(--ink)' }}>{p[0]}</b> {p[2]}</span>) };
  }
  return { w, cap: shownCap, label: '', cls: '' };
}

/** The n-th global key of an action, as a <kbd>. */
const kbds = (a: Action, n: number) => { const k = keysFor(a)[n]; return k ? <kbd>{keyName(k)}</kbd> : null; };

const LAYERS: [OverlayLayer, string][] = [['plain', 'Keys'], ['shift', 'Shift'], ['ctrl', 'Ctrl'], ['leader', 'Space leader']];

export function KeyboardOverlay() {
  const ui = useUi();
  const ov = ui.overlay;
  if (!ov) return null;
  const scope = SCOPES[ov.scope];
  const set = (patch: Partial<Overlay>) => setUi({ overlay: { ...ov, ...patch } });
  const seqs = bindingsFor(ov.scope).filter(b => b[2] && ((b[0].length > 1 && !b[0].startsWith('<') && !b[0].startsWith('C-')) || b[0].startsWith('<')));
  return (
    <div className="overlay">
      <div className="ov-head">
        <h2>Keyboard · {scope?.name}</h2><span className="dim">{scope?.hint}</span><span className="spacer" />
        <span className="dim"><kbd>{OVERLAY_KEYS.prevScope[0]}</kbd> <kbd>{OVERLAY_KEYS.nextScope[0]}</kbd> view · <kbd>{keyName(OVERLAY_KEYS.nextLayer[0] ?? '')}</kbd> layer · <kbd>{keyName(OVERLAY_KEYS.close[0] ?? '')}</kbd> close</span>
        <button className="chip" onClick={() => setUi({ overlay: null })}>✕ close</button>
      </div>
      <div className="ov-head">
        <div className="seg">{Object.entries(SCOPES).map(([k, v]) => <button key={k} className={ov.scope === k ? 'on' : ''} onClick={() => set({ scope: k })}>{v.name}</button>)}</div>
        <div className="seg">{LAYERS.map(([k, n]) => <button key={k} className={ov.layer === k ? 'on' : ''} onClick={() => set({ layer: k, leader: [] })}>{n}</button>)}</div>
      </div>
      <div className="legend">
        <span className="lg-move">move</span><span className="lg-act">act on it</span><span className="lg-view">open / switch view</span>
        <span className="lg-mode">mode</span><span className="lg-group">leader group</span><span className="lg-none">● = only in this view; the rest work everywhere</span>
      </div>
      {ov.layer === 'leader' && (
        <div style={{ margin: '4px 0' }}>Showing: <kbd>Space</kbd>{ov.leader.map(k => <span key={k}> <kbd>{k}</kbd></span>)}{' '}
          {ov.leader.length
            ? <button className="chip" onClick={() => set({ leader: ov.leader.slice(0, -1) })}>↑ up a level</button>
            : <span className="dim">click a group key to see inside it</span>}
        </div>
      )}
      <div className="kb">
        {KB_ROWS.map((row, r) => (
          <div className="kb-row" key={r}>
            {row.map((def, i) => {
              const k = keyInfo(def, ov);
              return (
                <div key={i} className={`key ${k.w} ${k.cls}`} onClick={k.group ? () => set({ leader: [...ov.leader, k.group ?? ''] }) : undefined}>
                  <span className="cap">{k.cap}</span><span className="lab">{k.label}</span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <div className="ov-cols">
        <div className="ov-box"><h3>Sequences and special keys here</h3>
          {seqs.length ? seqs.map(b => <div key={b[0]}><kbd>{b[0]}</kbd> {b[2]}</div>) : <div className="dim">none</div>}
        </div>
        <div className="ov-box"><h3>Space leader</h3>
          <div className="leader-tree">
            {Object.entries(LEADER).map(([k, v]) => Array.isArray(v)
              ? <div className="grp" key={k}><kbd>Space {k === '<Space>' ? 'Space' : k}</kbd> {v[1]}</div>
              : <div className="grp" key={k}><div><kbd>Space {k}</kbd> <b>{v.name}</b></div>
                  {Object.entries(v.keys).map(([k2, l]) => <div key={k2} style={{ paddingLeft: 18 }}><kbd>{k2}</kbd> {l[1]}</div>)}</div>)}
          </div>
        </div>
        <div className="ov-box"><h3>Zones</h3>
          <div>{kbds('zoneLeft', 0)}/{kbds('zoneRight', 0)} move between chat, main view and sidebar (left to right) where the view has no use for them; {kbds('zoneLeft', 1)}/{kbds('zoneRight', 1)} always do (also in the canvas and while typing).</div>
          <div>{kbds('zoneDown', 0)}/{kbds('zoneUp', 0)} inbox ⇄ efforts tree, canvas ⇄ incoming strip.</div>
          <div>{kbds('nextEffort', 0)}/{kbds('prevEffort', 0)} next or previous effort from anywhere; the "where was I" card comes up.</div>
          <div>Mouse: everything is clickable, nothing needs it.</div>
        </div>
      </div>
    </div>
  );
}
