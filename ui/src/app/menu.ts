import { api } from '../api';
import { errorToast, toast, trigger } from '../lib/ui';
import type { View } from '../state';
import { transport } from '../transport';
import { openComposer } from '../views/mail/composer';
import { toggleTheme } from './help';

/** What the menu bar needs from the shell. */
export interface MenuHost {
  go(view: View): void;
}

/** Runs the desktop menu bar's items. Most are a shortcut's binding
 *  (`key:r` does what pressing r does), so menu and keyboard can't drift. */
export function installMenu(host: MenuHost) {
  const actions: Record<string, () => void> = {
    compose: () => openComposer({}),
    'new-event': () => { host.go('calendar'); requestAnimationFrame(() => trigger('n')); },
    'new-contact': () => { host.go('contacts'); requestAnimationFrame(() => trigger('n')); },
    sync: () => api.syncNow(null).then(() => toast('Syncing…', { quiet: true }), errorToast),
    theme: () => toggleTheme(),
  };
  transport.onMenu((id) => {
    if (id.startsWith('key:')) trigger(id.slice(4));
    else if (id.startsWith('go:')) host.go(id.slice(3) as View);
    else actions[id]?.();
  });
}
