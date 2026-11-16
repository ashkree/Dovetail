// New cards from agents land here first: a, place on the canvas; x, drop.

import { dispatch } from '../../keys/actions';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { CardView } from './CardView';

export function IncomingStrip({ canvasId }: { canvasId: string }) {
  const s = useSnapshot();
  const ui = useUi();
  const incoming = s.incoming.filter(c => c.canvas_id === canvasId);
  const focus = ui.zone === 'main' && ui.canvasPart === 'incoming';
  return (
    <div className={`incoming ${focus ? 'focus' : ''}`}>
      <span className="lbl">incoming · {incoming.length}</span>
      {incoming.map((c, i) => (
        <div key={c.id} onClick={() => dispatch('selectIncoming', i)}>
          <CardView card={c} sel={focus && i === ui.incSel} under={c.under} />
        </div>
      ))}
      {!incoming.length && <span className="dim" style={{ alignSelf: 'center' }}>Nothing new. New cards from agents land here first.</span>}
    </div>
  );
}
