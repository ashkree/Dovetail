import { useUi } from '../../state/ui';
import { MainView } from '../main/MainView';
import { Sidebar } from '../sidebar/Sidebar';
import { Chat } from '../chat/Chat';

/** The three zones: chat · main view · sidebar. Without a main view the chat takes its space. */
export function Workspace() {
  const ui = useUi();
  const cls = `ws ${ui.main ? '' : 'nomain'} ${ui.sideCollapsed ? 'railed' : ''}`;
  return (
    <div className={cls} style={ui.sideCollapsed ? undefined : { ['--side-w' as string]: `${ui.sideW}px` }}>
      <Chat />
      <MainView />
      <Sidebar />
    </div>
  );
}
