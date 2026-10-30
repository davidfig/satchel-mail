import './styles/index.css';

import { transport } from './transport';
import { h } from './lib/dom';
import { installTooltips } from './lib/tooltip';
import { installShortcuts } from './lib/ui';
import { store } from './state';
import { restoreLocation } from './app/location';
import { Shell } from './app/shell';
import { followPage } from './app/follow';
import { catchHostLinks, followLink } from './views/mail/message-body';
import { followTarget, linkScheme } from './lib/thread';

async function boot(): Promise<boolean> {
  installShortcuts();
  installTooltips();
  catchHostLinks();
  if (transport.kind === 'tauri' && /Mac/.test(navigator.platform)) {
    // Overlay title bar: leave room for the window controls and make the strip draggable.
    document.body.classList.add('tauri-mac');
    document.body.appendChild(h('div.titlebar-drag', { 'data-tauri-drag-region': '' }));
    // Full screen hides the window controls, so it needs no room for them.
    transport.onFullscreen((on) => document.body.classList.toggle('fullscreen', on));
  }
  try {
    await store.init();
  } catch (e) {
    document.body.append(h('div.fatal', h('h1', 'Can’t reach satchel.email'), h('p', e instanceof Error ? e.message : String(e)), h('button.primary', { on: { click: () => location.reload() } }, 'Retry')));
    return false;
  }
  if (store.state.accounts.length) restoreLocation();
  new Shell();
  if (!store.state.accounts.length) store.set({ view: 'settings' });
  return true;
}

// A link from a message (web only; see `routeLinks`). Only `#follow=` is read
// here; `#session=` is the transport's.
const follow = transport.kind === 'http' ? followTarget(location.href, location.origin) : null;
if (transport.kind === 'http') {
  // A link opened in a tab already on this page changes only the fragment; reload to take it.
  window.addEventListener('hashchange', () => followTarget(location.href, location.origin) && location.reload());
}
if (follow) {
  // Taken off the address so a reload is the plain app.
  history.replaceState(null, '', location.pathname + location.search);
  // An email link opens the composer, which needs the app; it sends nothing on its own.
  if (linkScheme(follow.url) === 'mail') void boot().then((ok) => ok && followLink(follow.url, follow.cautious));
  else void followPage(follow.url, follow.cautious);
} else void boot();
