import './styles/index.css';

import { transport } from './transport';
import { h } from './lib/dom';
import { installTooltips } from './lib/tooltip';
import { installShortcuts } from './lib/ui';
import { store } from './state';
import { restoreLocation } from './app/location';
import { Shell } from './app/shell';

async function boot() {
  installShortcuts();
  installTooltips();
  if (transport.kind === 'tauri' && /Mac/.test(navigator.platform)) {
    // Overlay title bar: leave room for the window controls and make the strip draggable.
    document.body.classList.add('tauri-mac');
    document.body.appendChild(h('div.titlebar-drag', { 'data-tauri-drag-region': '' }));
  }
  try {
    await store.init();
  } catch (e) {
    document.body.append(h('div.fatal', h('h1', 'Can’t reach Satchel Mail'), h('p', e instanceof Error ? e.message : String(e)), h('button.primary', { on: { click: () => location.reload() } }, 'Retry')));
    return;
  }
  if (store.state.accounts.length) restoreLocation();
  new Shell();
  if (!store.state.accounts.length) store.set({ view: 'settings' });
}

void boot();
