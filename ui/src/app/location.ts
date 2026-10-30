import type { MailboxRole } from '../types';
import { type MailSource, type View, store } from '../state';

export const UNIFIED: { role: MailboxRole; label: string; icon: string }[] = [
  { role: 'inbox', label: 'Inbox', icon: 'inbox' },
  { role: 'flagged', label: 'Flagged', icon: 'star' },
  { role: 'drafts', label: 'Drafts', icon: 'draft' },
  { role: 'sent', label: 'Sent', icon: 'send' },
  { role: 'archive', label: 'Archive', icon: 'archive' },
  { role: 'junk', label: 'Spam', icon: 'junk' },
  { role: 'trash', label: 'Trash', icon: 'trash' },
];

export const COLLAPSED_KEY = 'satchel.sidebarCollapsed';
export const SECTIONS_KEY = 'satchel.sidebarOpenSections';
export const LOCATION_KEY = 'satchel.lastLocation';

export function readJson<T>(key: string): T | null {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null') as T | null;
  } catch {
    return null;
  }
}

export function writeJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* remembering is a convenience */
  }
}

/** Reopen where the user left off, unless that mailbox has since gone away. */
export function restoreLocation() {
  const last = readJson<{ view: View; source: MailSource }>(LOCATION_KEY);
  if (!last || !['mail', 'calendar', 'contacts', 'review', 'settings'].includes(last.view)) return;
  const source = last.source?.kind === 'mailbox' && !store.mailbox(last.source.mailbox_id) ? store.state.source : last.source ?? store.state.source;
  store.set({ view: last.view, source });
}

export function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === '1';
  } catch {
    return false;
  }
}
