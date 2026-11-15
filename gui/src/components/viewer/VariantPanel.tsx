// The variant panel: global, component and content slots. Mix and match, detach (floats, drags),
// close, and "Keep this combination", which tells the slice to make the mix permanent.

import type { MouseEvent } from 'react';
import type { VariantLevel, VariantSet } from '../../core/model';
import { dispatch } from '../../keys/actions';
import { setUi, uiStore, useUi } from '../../state/ui';
import { keyText } from '../overlays/Keys';

const LEVEL: Record<VariantLevel, string> = { global: 'Global', component: 'Component', content: 'Content' };

function startDrag(e: MouseEvent): void {
  if ((e.target as HTMLElement).closest('button')) return;
  e.preventDefault();
  const { panelPos } = uiStore.get();
  const sx = e.clientX - panelPos.x, sy = e.clientY - panelPos.y;
  const move = (ev: globalThis.MouseEvent) => setUi({ panelPos: { x: ev.clientX - sx, y: ev.clientY - sy } });
  const up = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
  window.addEventListener('mousemove', move);
  window.addEventListener('mouseup', up);
}

export function VariantPanel({ set }: { set: VariantSet }) {
  const ui = useUi();
  if (ui.panel === 'closed') {
    return <button className="chip panel-closed" onClick={() => dispatch('togglePanel')}>variants closed · <kbd>{keyText('togglePanel', 'viewer')}</kbd> opens</button>;
  }
  const detached = ui.panel === 'detached';
  const same = set.slots.every((_, i) => (ui.choice[i] ?? 0) === (set.kept[i] ?? 0));
  return (
    <div className={`vpanel ${detached ? 'detached' : ''}`} style={detached ? { left: ui.panelPos.x, top: ui.panelPos.y } : undefined}>
      <div className="vh" onMouseDown={detached ? startDrag : undefined}>
        Variants<span className="spacer" />
        <button onClick={() => dispatch('detachPanel')}>{detached ? '⇲ dock' : '⇱ detach'}</button>
        <button onClick={() => dispatch('closePanel')}>✕</button>
      </div>
      {set.slots.map((slot, i) => (
        <div key={slot.id}>
          {(i === 0 || set.slots[i - 1]?.level !== slot.level) && <div className="lvl">{LEVEL[slot.level]}</div>}
          <div className={`slot ${ui.slot === i ? 'sel' : ''}`} onClick={() => dispatch('selectSlot', i)}>
            <div className="sn">{slot.name}</div>
            <div className="opts">
              {slot.options.map((o, j) => (
                <button key={o} className={`opt ${(ui.choice[i] ?? 0) === j ? 'on' : ''} ${(set.kept[i] ?? 0) === j ? 'kept' : ''}`}
                  onClick={e => { e.stopPropagation(); dispatch('pickOption', [i, j]); }}>
                  <span className="n">{j + 1}</span>{o}
                </button>
              ))}
            </div>
          </div>
        </div>
      ))}
      <button className={`keep ${same ? 'same' : ''}`} onClick={() => dispatch('keepCombo')}>
        {same ? 'This is the kept combination' : `Keep this combination  (${keyText('keepCombo', 'viewer')})`}
      </button>
    </div>
  );
}
