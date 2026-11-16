// The viewer: the focused slice's web preview with the native variant panel beside it. The preview
// is an iframe of public/sample/; the chosen mix is posted to it, so changing a variant changes the page.

import { useEffect, useRef } from 'react';
import { dispatch, viewerSet } from '../../keys/actions';
import { useSnapshot } from '../../state/store';
import { useUi } from '../../state/ui';
import { keyText } from '../overlays/Keys';
import { VariantPanel } from './VariantPanel';

const SAMPLE = '/sample/index.html';

export function Viewer() {
  const s = useSnapshot();
  const ui = useUi();
  const set = viewerSet(s, ui.effort);
  const frame = useRef<HTMLIFrameElement>(null);
  const agent = s.agents.find(a => a.id === set?.agent_id);

  const post = () => {
    if (!set) return;
    const choice: Record<string, number> = {};
    const text: Record<string, string> = {};
    set.slots.forEach((slot, i) => {
      const j = ui.choice[i] ?? 0;
      choice[slot.id] = j;
      text[slot.id] = slot.options[j] ?? '';
    });
    frame.current?.contentWindow?.postMessage({ type: 'dovetail-variants', choice, text }, location.origin);
  };
  useEffect(post);

  if (!set) return <div className="empty-main">No slice in this project has a web preview yet.</div>;
  return (
    <div className="viewer">
      <div className="preview">
        <div className="urlbar">
          <span>◆ {agent?.name ?? set.agent_id}{agent?.branch ? ` · ${agent.branch}` : ''}</span>
          <span className="mono">{set.url}</span><span className="spacer" />
          <button onClick={() => dispatch('reload')}>⟳ {keyText('reload', 'viewer')}</button>
          <button onClick={() => dispatch('browser')}>↗ {keyText('browser', 'viewer')}</button>
        </div>
        <iframe ref={frame} key={ui.previewLoads} src={SAMPLE} title={`Preview of ${agent?.name ?? set.agent_id}`} onLoad={post} />
      </div>
      <VariantPanel set={set} />
    </div>
  );
}
