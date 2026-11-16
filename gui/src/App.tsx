import { TopBar } from './components/shell/TopBar';
import { BottomBar } from './components/shell/BottomBar';
import { Workspace } from './components/shell/Workspace';
import { WhichKey } from './components/overlays/WhichKey';
import { KeyboardOverlay } from './components/overlays/KeyboardOverlay';
import { Picker } from './components/overlays/Picker';
import { Toast } from './components/overlays/Toast';
import { useKeys } from './keys/useKeys';
import { WhereWasIPopup } from './components/main/WhereWasI';
import { Home } from './components/home/Home';
import { useUi } from './state/ui';

export function App() {
  useKeys();
  const { screen } = useUi();
  return (
    <div className="app">
      <TopBar />
      {screen === 'home' ? <Home /> : <Workspace />}
      <BottomBar />
      <WhereWasIPopup />
      <WhichKey />
      <KeyboardOverlay />
      <Picker />
      <Toast />
    </div>
  );
}
