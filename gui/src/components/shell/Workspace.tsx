import { useUi } from '../../state/ui';
import { MainView } from '../main/MainView';
import { Sidebar } from '../sidebar/Sidebar';

/** The three zones: chat · main view · sidebar. Without a main view the chat takes its space. */
export function Workspace() {
  const ui = useUi();
  const cls = `ws ${ui.main ? '' : 'nomain'} ${ui.sideCollapsed ? 'railed' : ''}`;
  return (
    <div className={cls} style={ui.sideCollapsed ? undefined : { ['--side-w' as string]: `${ui.sideW}px` }}>
      <div className={`zone ${ui.zone === 'chat' ? 'focus' : ''}`}><div className="zone-title">Coordinator · Shop</div></div>
      <MainView />
      <Sidebar />
    </div>
  );
}
