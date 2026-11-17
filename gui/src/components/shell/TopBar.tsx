import { useSnapshot } from '../../state/store';
import { useUi, type MainView } from '../../state/ui';
import { dispatch } from '../../keys/actions';

const TABS: [MainView | null, string, string][] = [
  [null, 'Chat', 'Space v x'], ['canvas', 'Canvas', 'Space v c'], ['viewer', 'Viewer', 'Space v w'], ['reading', 'Reading', 'Space v r'],
];

/** Brand, `Projects / <project>`, the project's repos, the view tabs. Nothing else. */
export function TopBar() {
  const s = useSnapshot();
  const ui = useUi();
  const project = s.projects.find(p => p.id === s.open_project);
  const inWs = ui.screen === 'ws' && project;
  // An inbox item and an agent's conversation have a tab only while open.
  const extra: [MainView, string, string][] = ui.main === 'item' ? [['item', 'Inbox item', 'Enter in the inbox']]
    : ui.main === 'agent' ? [['agent', s.agents.find(a => a.id === ui.agent)?.name ?? 'Agent', 'c in the tree']] : [];
  const tabs = [...TABS, ...extra];
  return (
    <div className="header">
      <span className="brand">◆ Dovetail</span>
      {inWs ? <>
        <span className="crumb"><button className="dim" onClick={() => dispatch('home')}>Projects</button><span className="sep">/</span>{project.name}</span>
        {project.repos.map(r => <span key={r} className="chip repo">{r}</span>)}
        <nav className="viewtabs">
          {tabs.map(([view, name, key]) => (
            <button key={name} className={`vtab ${ui.main === view ? 'on' : ''}`} title={key}
              onClick={() => dispatch('showView', view)}>{name}</button>
          ))}
        </nav>
      </> : <span className="crumb">Projects</span>}
      <span className="spacer" />
    </div>
  );
}
