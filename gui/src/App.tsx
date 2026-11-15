import { TopBar } from './components/shell/TopBar';
import { BottomBar } from './components/shell/BottomBar';
import { Workspace } from './components/shell/Workspace';
import { WhichKey } from './components/overlays/WhichKey';
import { KeyboardOverlay } from './components/overlays/KeyboardOverlay';
import { Picker } from './components/overlays/Picker';
import { Toast } from './components/overlays/Toast';
import { useKeys } from './keys/useKeys';

export function App() {
  useKeys();
  return (
    <div className="app">
      <TopBar />
      <Workspace />
      <BottomBar />
      <WhichKey />
      <KeyboardOverlay />
      <Picker />
      <Toast />
    </div>
  );
}
