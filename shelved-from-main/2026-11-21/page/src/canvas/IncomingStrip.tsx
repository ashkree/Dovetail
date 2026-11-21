// Cards an agent put in no block wait here: a places one (the core finds the spot); x drops it.
// It shows whether or not the effort has blocks yet (ADR 0027).

import { dispatch } from '../../keys/actions';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { CardView } from './CardView';

export function IncomingStrip({ effortId }: { effortId: string | null }) {
  const s = useSnapshot();
  const ui = useUi();
  const incoming = s.incoming.filter(c => c.effort_id === effortId);
  const focus = ui.zone === 'main' && ui.canvasPart === 'incoming';
  return (
    <div className={`incoming ${focus ? 'focus' : ''}`}>
      <span className="lbl">incoming · {incoming.length}</span>
      {incoming.map((c, i) => (
        <div key={c.id} onClick={() => dispatch('selectIncoming', i)}>
          <CardView card={c} sel={focus && i === ui.incSel} under={c.under} />
        </div>
      ))}
      {!incoming.length && <span className="dim" style={{ alignSelf: 'center' }}>Nothing waiting. Cards an agent puts in no block land here.</span>}
    </div>
  );
}
