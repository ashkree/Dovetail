// UI state in the URL hash, as the mock does: `#screen="ws"&main="canvas"&effort="pay"` opens any
// screen directly (for screenshots and comparison). Values are JSON, or plain strings. On load every
// key that names a UI field is applied; `project` names the project to open (default: the first).
// While running, the place you are (screen, project, view, effort) is written back without a reload.

import { dispatch } from '../keys/actions';
import { snapshotStore } from './store';
import { initialUi, setUi, uiStore, type Ui } from './ui';

const WRITE: (keyof Ui)[] = ['screen', 'main', 'effort'];


export function applyHash(hash: string): void {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const patch: Record<string, unknown> = {};
  let project: string | null = null;
  for (const [k, raw] of params) {
    let v: unknown = raw;
    try { v = JSON.parse(raw); } catch { /* a plain string */ }
    if (k === 'project') project = String(v);
    else if (k in initialUi) patch[k] = v;
  }
  if (patch.screen === 'ws' || project) {
    const i = Math.max(0, snapshotStore.get().projects.findIndex(p => p.id === project));
    dispatch('openProject', i);
  }
  if (typeof patch.effort === 'string' && !('tag' in patch)) patch.tag = patch.effort;
  setUi(patch as Partial<Ui>);
}

/** Keep the hash in step with where the user is. */
export function followHash(): () => void {
  let last = '';
  return uiStore.subscribe(() => {
    const u = uiStore.get();
    const s = snapshotStore.get();
    const parts = WRITE.filter(k => u[k] !== null && u[k] !== initialUi[k] || k === 'screen')
      .map(k => `${k}=${show(u[k])}`);
    if (u.screen === 'ws' && s.open_project) parts.splice(1, 0, `project=${show(s.open_project)}`);
    const next = '#' + parts.join('&');
    if (next !== last) { last = next; history.replaceState(null, '', next); }
  });
}

/** Strings plainly (`screen=ws`), anything else as JSON. */
function show(v: unknown): string {
  return encodeURIComponent(typeof v === 'string' ? v : JSON.stringify(v));
}
