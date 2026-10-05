import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import 'dockview-react/dist/styles/dockview.css';
import './styles/app.css';

const root = document.getElementById('root');
if (!root) throw new Error('Application root is missing.');

createRoot(root).render(<StrictMode><App /></StrictMode>);
