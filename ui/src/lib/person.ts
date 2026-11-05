// People: avatars and links that open everything exchanged with someone.

import { api } from '../api';
import { store } from '../state';
import type { MailSource } from '../state';
import type { Address } from '../types';
import { h } from './dom';
import { avatarColor, displayName, initials, shortName } from './format';
import { errorToast, toast } from './ui';

export function avatar(a: Address | null | undefined, big = false) {
  return h(`span.avatar${big ? '.big' : ''}`, { style: { background: avatarColor(a?.email ?? '') }, 'aria-hidden': 'true' }, initials(a));
}

/** Show this person's conversations in the mail list, keeping the open
 *  conversation. Give all of their addresses, or one: the core adds the rest
 *  of their contact's addresses. */
export function openPerson(addresses: Address | Address[], label?: string) {
  const list = Array.isArray(addresses) ? addresses : [addresses];
  if (!list.length) return;
  const cur = store.state.source;
  const back: MailSource = cur.kind === 'person' ? cur.back ?? { kind: 'role', role: 'inbox', label: 'Inbox' } : cur;
  const emails = [...new Set(list.map((a) => a.email.toLowerCase()))];
  store.set({ view: 'mail', source: { kind: 'person', emails, label: label ?? displayName(list[0]), filter: 'all', back } });
}

/** A name that opens the person view. */
export function personLink(a: Address | null | undefined, text = displayName(a)) {
  if (!a) return h('span', text);
  return h('button.person-link', { title: `All mail with ${a.email}`, on: { click: (e: MouseEvent) => { e.stopPropagation(); openPerson(a); } } }, text);
}

/** An avatar that opens the person view. */
export function avatarLink(a: Address | null | undefined) {
  if (!a) return avatar(a);
  return h('button.avatar-link', { title: `All mail with ${a.email}`, 'aria-label': `All mail with ${displayName(a)}`, on: { click: (e: MouseEvent) => { e.stopPropagation(); openPerson(a); } } }, avatar(a));
}

/** Whether all of these addresses are favorite senders (0053). */
export function isFavorite(addresses: Address[]): boolean {
  return addresses.length > 0 && addresses.every((a) => store.state.favorites.has(a.email.toLowerCase()));
}

/** Add one person (all the addresses given) to the favorites, or take them off. */
export async function setFavorite(addresses: Address[], favorite: boolean) {
  if (!addresses.length) return;
  try {
    await api.setFavoriteSenders(addresses.map((a) => a.email), favorite);
    const who = shortName(addresses[0]);
    toast(favorite ? `${who} is a favorite. Their mail is highlighted.` : `${who} is no longer a favorite.`);
  } catch (e) {
    errorToast(e);
  }
}

/** A menu or palette entry that toggles whether this person is a favorite. */
export function favoriteItem(addresses: Address[]) {
  const fav = isFavorite(addresses);
  const who = shortName(addresses[0]);
  return { label: fav ? `Remove ${who} from favorites` : `Add ${who} to favorites`, icon: 'heart', run: () => void setFavorite(addresses, !fav) };
}
