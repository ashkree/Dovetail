import { dispatch } from '../../keys/actions';
import { useSnapshot } from '../../state/store';
import { needsYou } from '../../state/select';
import { useUi } from '../../state/ui';
import { tagColour } from '../chat/EffortTag';
import { keyText } from '../overlays/Keys';

/** The folded sidebar: the inbox count and one dot per effort. */
export function Rail() {
  const s = useSnapshot();
  const { effort } = useUi();
  return (
    <div className="zone sidebar rail">
      <button className="rail-inbox" title={`Inbox (${keyText('focusInbox')})`} onClick={() => dispatch('focusInbox')}>
        {needsYou(s) ? <><span className="needs">⚠</span>&nbsp;{needsYou(s)}</> : '✓'}
      </button>
      {s.efforts.map(e => (
        <button key={e.id} className={`rail-effort ${e.id === effort ? 'current' : ''}`} title={e.name} onClick={() => dispatch('switchEffort', e.id)}>
          <span className="dot" style={{ background: tagColour(e) }} />
        </button>
      ))}
      <span className="spacer" />
      <button className="dim" title={`Expand the sidebar (${keyText('toggleSide')})`} onClick={() => dispatch('toggleSide')}>⇤</button>
    </div>
  );
}
