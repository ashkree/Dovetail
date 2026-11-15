import { useUi } from '../../state/ui';

/** The three zones: chat · main view · sidebar. Without a main view the chat takes its space. */
export function Workspace() {
  const ui = useUi();
  const f = (z: string) => ui.zone === z ? 'focus' : '';
  return (
    <div className={`ws ${ui.main ? '' : 'nomain'}`} style={{ ['--side-w' as string]: `${ui.sideW}px` }}>
      <div className={`zone ${f('chat')}`}><div className="zone-title">Coordinator · Shop</div></div>
      {ui.main && <div className={`zone ${f('main')}`}><div className="zone-title">{ui.main}</div></div>}
      <div className="sidebar">
        <div className={`part ${f('inbox')}`}><div className="zone-title">Inbox</div></div>
        <div className={`part ${f('tree')}`}><div className="zone-title">Efforts</div></div>
      </div>
    </div>
  );
}
