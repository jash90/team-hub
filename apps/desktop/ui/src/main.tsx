import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

// The look of the web interface, then what the desktop adds to it.
import '@web/styles.css';
import './desktop.css';
import './i18n.ts';
import { App } from './app.tsx';

// The theme chosen before; the same key as the web interface.
try {
  const theme = window.localStorage.getItem('team-hub:theme');

  if (theme === 'dark' || theme === 'light') {
    document.documentElement.dataset.theme = theme;
  }
} catch {
  // Without storage the system's theme applies.
}

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
