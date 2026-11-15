// The main view zone: whichever view is open (canvas, viewer, reading, an inbox item).

import { dispatch } from '../../keys/actions';
import { useUi, type MainView as View } from '../../state/ui';
import { LeaderKey } from '../overlays/Keys';
import { InboxItemView } from './InboxItemView';
import { ReadingView } from './ReadingView';
import { Viewer } from '../viewer/Viewer';
import { Canvas } from '../canvas/Canvas';

const TITLE: Record<View, string> = { canvas: 'Canvas', viewer: 'Viewer', reading: 'Reading', item: 'Inbox item' };

export function MainView() {
  const ui = useUi();
  if (!ui.main) return null;
  return (
    <div className={`zone ${ui.zone === 'main' ? 'focus' : ''}`}>
      <div className="zone-title" onClick={() => dispatch('focusZone', 'main')}>
        <span>{TITLE[ui.main]}</span><span className="spacer" />
        <button className="dim" title="Close the view; the chat takes the space" onClick={e => { e.stopPropagation(); dispatch('closeMain'); }}>
          ✕ close <LeaderKey action="closeMain" />
        </button>
      </div>
      <div className="main-body">
        {ui.main === 'item' && <InboxItemView />}
        {ui.main === 'reading' && <ReadingView />}
        {ui.main === 'viewer' && <Viewer />}
        {ui.main === 'canvas' && <Canvas />}
      </div>
    </div>
  );
}
