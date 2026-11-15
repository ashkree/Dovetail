import { TopBar } from './components/shell/TopBar';
import { BottomBar } from './components/shell/BottomBar';
import { Workspace } from './components/shell/Workspace';

export function App() {
  return (
    <div className="app">
      <TopBar />
      <Workspace />
      <BottomBar />
    </div>
  );
}
