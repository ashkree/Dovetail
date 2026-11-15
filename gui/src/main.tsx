import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/atkinson-hyperlegible/400.css';
import '@fontsource/atkinson-hyperlegible/400-italic.css';
import '@fontsource/atkinson-hyperlegible/700.css';
import '@fontsource/atkinson-hyperlegible/700-italic.css';
import './styles/tokens.css';
import './styles/base.css';
import './styles/shell.css';
import './styles/overlays.css';
import { App } from './App';
import { MockCore } from './core/mock';
import { connect } from './state/store';
import { dispatch } from './keys/actions';

connect(new MockCore());
dispatch('openProject', 0);

const root = document.getElementById('root');
if (!root) throw new Error('index.html has no #root');
createRoot(root).render(<StrictMode><App /></StrictMode>);
