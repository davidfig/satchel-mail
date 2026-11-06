// System notifications for new mail and for accounts that get signed out. The
// core decides what counts as new (see `CoreEvent::NewMail`); this decides
// whether to show it and how.

import { store } from '../state';
import { transport } from '../transport';
import type { NewMail } from '../types';
import { newlySignedOut, signOutMessage } from './signouts';

/** How long an account must stay signed out before we notify, so a brief
 *  refusal the next pass clears doesn't reach the user. */
const SIGN_OUT_SETTLE_MS = 60_000;

/** Notifies once each time an account gets signed out, unless the user is
 *  looking at the app, where the banner already says so. */
export function watchSignOuts() {
  const seen = new Set<string>();
  const check = () => {
    for (const a of newlySignedOut(store.state.accounts, seen)) setTimeout(() => void notifySignOut(a.id), SIGN_OUT_SETTLE_MS);
  };
  store.subscribe((_, changed) => {
    if (changed.has('accounts')) check();
  });
  check();
}

async function notifySignOut(accountId: string) {
  const a = store.state.accounts.find((x) => x.id === accountId);
  if (a?.sync.state !== 'sign_in' || document.hasFocus() || !(await transport.canNotify(false))) return;
  transport.notify(`Sign in again: ${a.email}`, signOutMessage(a), undefined, `satchel-sign-out-${a.id}`);
}

/** Shows a notification for mail that just arrived, unless the user is looking at the app. */
export async function notifyNewMail(messages: NewMail[]) {
  if (!messages.length || !store.state.settings?.notify_new_mail || document.hasFocus()) return;
  if (!(await transport.canNotify(false))) return;
  // The address too: anyone can name themselves "Apple Support", and an OS
  // alert carries no other sign of who it's from.
  const sender = (m: NewMail) => {
    const f = m.from;
    if (!f?.email) return f?.name || 'Unknown sender';
    return f.name && f.name.toLowerCase() !== f.email.toLowerCase() ? `${f.name} <${f.email}>` : f.email;
  };
  const open = (thread: string | null) =>
    store.set({ view: 'mail', source: { kind: 'role', role: 'inbox', label: 'Inbox' }, selectedThread: thread });
  if (messages.length === 1) {
    const m = messages[0];
    transport.notify(sender(m), [m.subject || '(no subject)', m.snippet].filter(Boolean).join('\n'), () => open(m.thread_id));
  } else {
    const senders = [...new Set(messages.map(sender))];
    const body = senders.length > 3 ? `${senders.slice(0, 3).join(', ')} and ${senders.length - 3} more` : senders.join(', ');
    transport.notify(`${messages.length} new messages`, body, () => open(null));
  }
}
