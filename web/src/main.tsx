import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { fetchApplicationInfo } from './application/api';
import { ApplicationStore } from './application/store';
import { App } from './ui/App';
import './style.css';

const store = new ApplicationStore(fetchApplicationInfo);
void store.refresh();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App store={store} />
  </StrictMode>,
);
