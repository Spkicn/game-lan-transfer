import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import App from './App';
import './app.css';

const container = document.getElementById('root');
if (!(container instanceof HTMLElement)) {
  throw new Error('页面缺少挂载点 #root');
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
