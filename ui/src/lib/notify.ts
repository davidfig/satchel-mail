// System notifications for new mail. The core decides what counts as new
// (see `CoreEvent::NewMail`); this decides whether to show it and how.

import { store } from '../state';
import { transport } from '../transport';
import type { NewMail } from '../types';

/** Shows a notification for mail that just arrived, unless the user is looking at the app. */
export async function notifyNewMail(messages: NewMail[]) {
  if (!messages.length || !store.state.settings?.notify_new_mail || document.hasFocus()) return;
  if (!(await transport.canNotify(false))) return;
  const sender = (m: NewMail) => m.from?.name || m.from?.email || 'Unknown sender';
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
