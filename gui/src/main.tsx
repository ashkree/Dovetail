import './perf/hook';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/atkinson-hyperlegible/400.css';
import '@fontsource/atkinson-hyperlegible/400-italic.css';
import '@fontsource/atkinson-hyperlegible/700.css';
import '@fontsource/atkinson-hyperlegible/700-italic.css';
import './styles/tokens.css';
import './styles/base.css';
import './styles/shell.css';
import './styles/home.css';
import './styles/sidebar.css';
import './styles/main.css';
import './styles/chat.css';
import './styles/viewer.css';
import '@xyflow/react/dist/base.css';
import './styles/canvas.css';
import './styles/overlays.css';
import { App } from './App';
import { TauriCore, inTauri } from './core/tauri';
import { connect } from './state/store';
import { toast } from './state/ui';
import { applyHash, followHash } from './state/urlHash';

// Inside the app window, the real core in the app's own process (ADR 0023); in a plain browser,
// the fake one with its fake data, loaded only then.
if (import.meta.env.VITE_PERF) {
  // A perf build (docs/plan/gui-perf.md): the harness brings its own synthetic core.
} else if (inTauri()) {
  connect(new TauriCore(message => toast(message)));
} else {
  const [{ MockCore }, { pendingFor }] = await Promise.all([import('./core/mock'), import('./core/fixtures')]);
  connect(new MockCore(), pendingFor);
}
applyHash(location.hash);
window.addEventListener('hashchange', () => applyHash(location.hash));
followHash();

const root = document.getElementById('root');
if (!root) throw new Error('index.html has no #root');
createRoot(root).render(<StrictMode><App /></StrictMode>);

if (import.meta.env.VITE_PERF) {
  const { runPerf } = await import('./perf/harness');
  void runPerf(Number(import.meta.env.VITE_PERF_CHAT ?? 300), Number(import.meta.env.VITE_PERF_CARDS ?? 40));
}
