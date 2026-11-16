// The one window keydown listener. Order: an open modal (picker, prompt, `:`), then the `?`
// overlay, then the keymap through resolve() and the dispatcher.

import { useEffect } from 'react';
import { scopeOf, setUi, toast, uiStore, type Modal, type OverlayLayer, type Ui } from '../state/ui';
import { dispatch, PASS_THROUGH, runCommand } from './actions';
import { MODAL_KEYS, OVERLAY_KEYS, SCOPES } from './keymap';
import { leaderNode, resolve, tokenOf } from './resolve';

const LAYERS: OverlayLayer[] = ['plain', 'shift', 'ctrl', 'leader'];

export function useKeys(): void {
  useEffect(() => {
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

/** The picker's rows after its filter. */
export function pickerRows(m: Extract<Modal, { type: 'picker' }>) {
  const q = m.q.toLowerCase();
  return m.items.filter(it => it.text.toLowerCase().includes(q));
}

function onKey(e: KeyboardEvent): void {
  const u = uiStore.get();
  if (u.modal) { modalKey(e, u.modal); return; }
  if (u.overlay) { overlayKey(e, u); return; }

  const token = tokenOf(e, u.mode === 'insert');
  if (!token) return;
  const r = resolve(token, scopeOf(u), { pending: u.pending, leader: u.leader });
  if (r.kind === 'text') return;
  if (r.kind === 'action' && PASS_THROUGH.has(r.action)) return;
  e.preventDefault();
  setUi({ pending: r.state.pending, leader: r.state.leader });
  if (r.kind === 'none' && r.unbound) toast(`${r.unbound} is not bound`);
  if (r.kind === 'action') dispatch(r.action);
}

function modalKey(e: KeyboardEvent, m: Modal): void {
  const t = tokenOf(e, true);
  if (!t) return;
  if (MODAL_KEYS.close.includes(t)) { e.preventDefault(); setUi({ modal: null }); return; }
  if (m.type === 'form') { e.preventDefault(); return; }
  if (m.type === 'picker') {
    const rows = pickerRows(m);
    if (MODAL_KEYS.down.includes(t) || MODAL_KEYS.up.includes(t)) {
      e.preventDefault();
      const d = MODAL_KEYS.down.includes(t) ? 1 : -1;
      setUi({ modal: { ...m, sel: Math.max(0, Math.min(rows.length - 1, m.sel + d)) } });
    } else if (MODAL_KEYS.accept.includes(t)) {
      e.preventDefault();
      setUi({ modal: null });
      rows[m.sel]?.go();
    }
    return;
  }
  if (MODAL_KEYS.accept.includes(t)) {
    e.preventDefault();
    setUi({ modal: null });
    if (m.type === 'command') runCommand(m.value);
    else if (m.value.trim()) m.onDone(m.value);
  }
}

function overlayKey(e: KeyboardEvent, u: Ui): void {
  const ov = u.overlay;
  const t = tokenOf(e, false);
  if (!ov || !t) return;
  e.preventDefault();
  const names = Object.keys(SCOPES);
  const step = (list: string[], cur: string, d: number) => list[(list.indexOf(cur) + d + list.length) % list.length] ?? cur;
  if (OVERLAY_KEYS.close.includes(t) && ov.layer === 'leader' && ov.leader.length && t === '<Esc>') setUi({ overlay: { ...ov, leader: ov.leader.slice(0, -1) } });
  else if (OVERLAY_KEYS.close.includes(t)) setUi({ overlay: null });
  else if (OVERLAY_KEYS.nextScope.includes(t)) setUi({ overlay: { ...ov, scope: step(names, ov.scope, 1) } });
  else if (OVERLAY_KEYS.prevScope.includes(t)) setUi({ overlay: { ...ov, scope: step(names, ov.scope, -1) } });
  else if (OVERLAY_KEYS.nextLayer.includes(t) || OVERLAY_KEYS.prevLayer.includes(t)) {
    const layer = step(LAYERS, ov.layer, OVERLAY_KEYS.nextLayer.includes(t) ? 1 : -1) as OverlayLayer;
    setUi({ overlay: { ...ov, layer, leader: [] } });
  } else if (ov.layer === 'leader') {
    const node = leaderNode(ov.leader)?.keys[t];
    if (node && !Array.isArray(node)) setUi({ overlay: { ...ov, leader: [...ov.leader, t] } });
  }
}
