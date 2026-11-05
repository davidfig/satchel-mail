// Settings → Favorites: the senders whose mail is highlighted (0053).

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { personLink } from '../../lib/person';
import { errorToast, toast } from '../../lib/ui';
import { store } from '../../state';
import type { SettingsView } from './index';

export function favorites(view: SettingsView, el: HTMLElement) {
  const list = [...store.state.favorites].sort();
  const input = h('input', { type: 'email', placeholder: 'name@example.com', 'aria-label': 'Address to add' });
  const set = async (emails: string[], favorite: boolean) => {
    try {
      await api.setFavoriteSenders(emails, favorite);
      await store.refreshFavorites();
      // Redraws even after typing in the field (the store's redraw waits for unsaved edits).
      view.render();
    } catch (e) {
      errorToast(e);
    }
  };
  const add = () => {
    // Several at once, separated by commas or spaces.
    const emails = input.value.split(/[\s,;]+/).filter(Boolean);
    if (!emails.length) return;
    const bad = emails.find((e) => !/^[^@\s]+@[^@\s]+$/.test(e));
    if (bad) toast(`“${input.value.trim()}” isn’t an email address.`, { kind: 'error' });
    else void set(emails, true);
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') add();
  });
  replace(el,
    h('h1', 'Favorites'),
    h('p.muted', 'Conversations with mail from these senders get a heart and a tint in the list, and Favorites in the sidebar (g f) shows them all. Favorites stay on this device; your mail provider never sees them.'),
    h('div.row.picker', input, h('button', { title: 'Add this address to favorites', on: { click: add } }, icon('plus', 16), 'Add')),
    list.length
      ? h('ul.plain-list.favorites-list', list.map((email) => h('li',
          icon('heart', 14),
          personLink({ email }, email),
          h('button.link', { title: 'Stop highlighting mail from this sender', on: { click: () => void set([email], false) } }, 'Remove'))))
      : h('p.small.muted', 'No favorites yet. Use “Add to favorites” on a conversation or a person, or add an address above.'));
}
