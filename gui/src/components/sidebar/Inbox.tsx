import { useEffect, useRef } from 'react';
import { dispatch } from '../../keys/actions';
import { effortById, INBOX_GLYPH } from '../../state/select';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { tagColour } from '../chat/EffortTag';
import { keyText } from '../overlays/Keys';

export function Inbox() {
  const s = useSnapshot();
  const ui = useUi();
  const focus = ui.zone === 'inbox' ? 'focus' : '';
  const sel = useRef<HTMLDivElement>(null);
  useEffect(() => { sel.current?.scrollIntoView({ block: 'nearest' }); }, [ui.inboxSel, ui.zone]);
  const selIdx = Math.min(ui.inboxSel, s.inbox.length - 1);
  return (
    <div className={`part ${focus}`}>
      <div className={`zone-title ${focus}`} onClick={() => dispatch('focusZone', 'inbox')}>
        Inbox · {s.inbox.length}<span className="spacer" />
        <button className="dim collapse-btn" title={`Collapse the sidebar (${keyText('toggleSide')})`}
          onClick={e => { e.stopPropagation(); dispatch('toggleSide'); }}>⇥</button>
      </div>
      <div className="list">
        {s.inbox.length ? s.inbox.map((it, i) => {
          const e = effortById(s, it.effort_id);
          return (
            <div key={it.id} ref={i === selIdx ? sel : undefined} className={`row ${i === selIdx ? 'sel' : ''}`}
              onClick={ev => { dispatch('selectInbox', i); if (ev.detail >= 2) dispatch('openItem'); }}>
              <span className={`g-${it.kind}`}>{INBOX_GLYPH[it.kind]}</span>
              <span className="dot" style={{ background: tagColour(e) }} title={e?.name} />
              <span className="t">{it.title}</span><span className="tm">{it.at}</span>
            </div>
          );
        }) : <div className="row dim">Nothing needs you.</div>}
      </div>
    </div>
  );
}
