// Acting on conversations: archive, delete, spam, flags, moves, menus.

import { api } from '../../api';
import { h } from '../../lib/dom';
import { shortName } from '../../lib/format';
import { folderDialog } from '../../lib/folders';
import { openPerson } from '../../lib/person';
import { confirmDialog, errorToast, popupMenu, toast } from '../../lib/ui';
import type { Command, MenuItem } from '../../lib/ui';
import { store } from '../../state';
import type { Mailbox, MessageDetail, Thread, ThreadSummary, UnsubscribeEmail } from '../../types';
import { openExternal, viewSource } from './message-body';
import type { MailView } from './index';

/** Tooltip for Spam on mail already in Spam. */
export const BLOCK_TIP = 'Spam: delete, and always send this sender’s mail to Spam';
export const isJunk = (ids: string[]) => ids.some((id) => store.mailbox(id)?.role === 'junk');

export type Action = 'archive' | 'trash' | 'read' | 'unread' | 'spam' | 'not_spam' | 'block';

/** Right-click menu for a conversation in the list. With several selected
 *  and the click on one of them, it acts on all of them. */
export function rowMenu(view: MailView, t: ThreadSummary, at: { x: number; y: number }): (MenuItem | '-')[] {
  const threads = view.selected.has(t.thread_id) && view.selected.size > 1 ? [...view.selected.values()] : [t];
  const one = threads.length === 1;
  const n = one ? '' : ` ${threads.length} conversations`;
  const unread = threads.some((x) => x.unread_count > 0);
  const flagged = threads.every((x) => x.flagged);
  const spam = view.inSpam(threads);
  const me = store.state.myEmails;
  const person = one ? t.participants.find((a) => !me.has(a.email.toLowerCase())) : undefined;
  const open = async () => {
    if (store.state.selectedThread !== t.thread_id || !view.openThread) await view.showThread(t.thread_id);
  };
  return [
    ...(one
      ? [
          { label: 'Open', icon: 'mailOpen', hint: 'Enter', run: () => void view.showThread(t.thread_id) },
          '-' as const,
          { label: 'Reply', icon: 'reply', hint: 'r', run: () => void open().then(() => view.replyBox?.focus('reply')) },
          { label: 'Reply all', icon: 'replyAll', hint: 'a', run: () => void open().then(() => view.replyBox?.focus('reply_all')) },
          { label: 'Forward', icon: 'forward', hint: 'f', run: () => void open().then(() => view.reply('forward')) },
          '-' as const,
        ]
      : []),
    unread
      ? { label: `Mark${n} as read`, icon: 'mailOpen', hint: 'Shift+I', run: () => void view.act('read', threads) }
      : { label: `Mark${n} as unread`, icon: 'mail', hint: 'u', run: () => void view.act('unread', threads) },
    { label: flagged ? 'Unflag' : 'Flag', icon: 'star', hint: 's', run: () => void Promise.all(threads.filter((x) => x.flagged === flagged).map((x) => view.flagThread(x))) },
    '-',
    { label: `Archive${n}`, icon: 'archive', hint: 'e', run: () => void view.act('archive', threads) },
    { label: 'Move to…', icon: 'folder', hint: 'v', run: () => view.moveMenu(at, threads) },
    ...(spam
      ? [
          { label: 'Not spam', icon: 'inbox', hint: '!', run: () => void view.act('not_spam', threads) },
          { label: 'Spam: delete and block sender', icon: 'junk', run: () => void view.act('block', threads) },
        ]
      : [{ label: 'Mark as spam', icon: 'junk', hint: '!', run: () => void view.act('spam', threads) }]),
    { label: `Delete${n}`, icon: 'trash', hint: '#', danger: true, run: () => void view.act('trash', threads) },
    '-',
    ...(person ? [{ label: `All mail with ${shortName(person)}`, icon: 'user', run: () => openPerson(person) }] : []),
    { label: view.selected.has(t.thread_id) ? 'Deselect' : 'Select', icon: 'check', hint: 'x', run: () => view.toggleSelect(t) },
  ];
}

export function targetThreads(view: MailView): ThreadSummary[] {
  if (view.selected.size) return [...view.selected.values()];
  const sel = store.state.selectedThread;
  if (sel) {
    // The conversation on screen, even after a refresh dropped it from the
    // list; never the neighbour the cursor moved to.
    const t = view.list.find(sel) ?? (view.openSummary?.thread_id === sel ? view.openSummary : undefined);
    return t ? [t] : [];
  }
  const t = view.list.at(view.cursor);
  return t ? [t] : [];
}

export async function messageIds(view: MailView, threads: ThreadSummary[]): Promise<string[]> {
  if (threads.length === 1 && view.openThread?.thread_id === threads[0].thread_id) return view.openThread.messages.map((m) => m.id);
  const out: string[] = [];
  for (const t of threads) {
    const full = await api.getThread(t.thread_id);
    out.push(...full.messages.map((m) => m.id));
  }
  return out;
}

/** Whether the targeted conversations are in Spam (so "!" means Not spam). */
export function inSpam(view: MailView, threads = view.targetThreads()): boolean {
  return threads.length > 0 && threads.every((t) => isJunk(t.mailbox_ids));
}

/** Actions that take conversations out of the current list, and their toasts. */
const MOVED: Partial<Record<Action, string>> = { archive: 'Archived', trash: 'Moved to Trash', spam: 'Marked as spam.', not_spam: 'Moved to Inbox.', block: 'Deleted.' };

export async function act(view: MailView, kind: Action, threads = view.targetThreads()) {
  if (!threads.length) return;
  const moved = MOVED[kind];
  if (kind === 'trash') {
    const inTrash = threads.every((t) => t.mailbox_ids.every((m) => store.mailbox(m)?.role === 'trash'));
    if (inTrash && !(await confirmDialog('Delete forever?', 'These messages will be permanently deleted.', 'Delete', true))) return;
  }
  try {
    // Starts reading the open thread's ids synchronously, before removeNow closes it.
    const pending = view.messageIds(threads);
    if (moved) removeNow(view, threads);
    const ids = await pending;
    if (kind === 'archive') await api.archive(ids);
    if (kind === 'trash') await api.trash(ids);
    if (kind === 'read') await api.setFlags(ids, true, null);
    if (kind === 'unread') await api.setFlags(ids, false, null);
    let note = '';
    if (kind === 'spam' || kind === 'not_spam' || kind === 'block') {
      const r = kind === 'not_spam' ? await api.markNotSpam(ids) : await api.markSpam(ids);
      // Spam from the Spam folder: remember the sender, then delete.
      if (kind === 'block') await api.trash(ids);
      const who = r.senders.length === 1 ? r.senders[0] : `${r.senders.length} senders`;
      const removed = 'invites_removed' in r && r.invites_removed ? r.invites_removed : 0;
      const more = (r.also_moved ? ` ${r.also_moved} more message${r.also_moved === 1 ? '' : 's'} moved.` : '')
        + (removed ? ` Removed ${removed === 1 ? 'its invitation' : `${removed} invitations`} from your calendar; the organizer isn’t told.` : '');
      if (r.senders.length) note = kind !== 'not_spam' ? ` Future mail from ${who} goes to Spam.${more}` : ` Mail from ${who} will stay out of Spam.${more}`;
    }
    if (moved) toast(moved + note, note ? { ms: 6000 } : {});
  } catch (e) {
    errorToast(e);
  }
  // Re-read the window: confirms the removal, or brings rows back if it failed.
  void view.refresh();
}

/** Take moved conversations off screen before the server answers: close the
 *  reader if it shows one, and drop them from the list with the cursor staying
 *  at the same position, which now holds the next thread. */
function removeNow(view: MailView, threads: ThreadSummary[]) {
  const removed = new Set(threads.map((t) => t.thread_id));
  const pos = view.cursor >= 0 ? view.cursor : view.list.indexOf(threads[0].thread_id);
  const sel = store.state.selectedThread;
  if ((sel && removed.has(sel)) || (view.openThread && removed.has(view.openThread.thread_id))) {
    store.set({ selectedThread: null });
    view.renderEmptyReader();
  }
  view.selected.clear();
  view.renderHeader();
  view.list.remove(removed);
  view.cursor = pos < 0 ? -1 : Math.min(pos, view.list.total - 1);
  view.renderRows();
}

export async function flagThread(view: MailView, t: ThreadSummary) {
  try {
    const full = await api.getThread(t.thread_id);
    const target = full.messages[full.messages.length - 1];
    await api.setFlags(t.flagged ? full.messages.map((m) => m.id) : [target.id], null, !t.flagged);
    t.flagged = !t.flagged;
    view.renderRows();
  } catch (e) {
    errorToast(e);
  }
}

export async function unsubscribe(view: MailView, m: MessageDetail, inSpam: boolean) {
  const u = m.unsubscribe!;
  const who = m.from?.name || m.from?.email || 'this sender';
  const how: string | Node = u.one_click ? `Satchel will tell ${who} to stop sending you this mail.`
    : u.email ? unsubscribeEmailText(u.email, m)
    : `This opens ${who}'s unsubscribe page in your browser.`;
  const spamWarning = 'Unsubscribing from real spam tells the sender your address works. Only do this for senders you recognize; otherwise leave it in Spam.';
  const ok = inSpam
    ? await confirmDialog('Unsubscribe from mail in Spam?', typeof how === 'string' ? `${spamWarning} ${how}` : h('div', h('p', spamWarning), how), 'Unsubscribe', true)
    : await confirmDialog(`Unsubscribe from ${who}?`, how, 'Unsubscribe');
  if (!ok) return;
  try {
    const r = await api.unsubscribe(m.id);
    if (r.via === 'browser' && r.url) openExternal(r.url);
    const msg = r.via === 'browser' ? 'Finish unsubscribing in your browser.' : r.via === 'email' ? `Unsubscribe request sent to ${r.to}.` : `Unsubscribed from ${who}.`;
    const ids = view.openThread?.thread_id === m.thread_id ? view.openThread.messages.map((x) => x.id) : [m.id];
    const archive = () => void api.archive(ids).then(() => { toast('Archived'); void view.refresh(); }, errorToast);
    toast(msg, inSpam ? {} : { kind: 'success', action: { label: 'Archive', run: archive } });
  } catch (e) {
    errorToast(e);
  }
}

/** Exactly what an unsubscribe email will send, and from which account,
 *  since the sender wrote the address, subject and text. Shown in full:
 *  nothing is cut, so nothing is sent that you didn't see. */
function unsubscribeEmailText(e: UnsubscribeEmail, m: MessageDetail): HTMLElement {
  const account = store.state.accounts.find((a) => a.id === m.account_id)?.email ?? 'your account';
  const domain = m.from?.email.split('@').pop();
  return h('div',
    h('p', `Satchel will send this email from ${account} to ${e.to}:`),
    h('div.kv', h('span', 'Subject'), h('span.sent-preview-subject', e.subject || '(no subject)')),
    h('pre.draft-text.sent-preview', e.body),
    e.other_domain && h('p', `Note: ${e.to} isn’t at ${domain ?? 'the sender’s domain'}. Mailing list services often use their own domain, but only continue if you recognize it.`));
}

/** Folders and labels the conversations can move to. */
function moveTargets(accountId: string): Mailbox[] {
  return store.state.mailboxes.filter((m) => m.account_id === accountId && m.role !== 'all' && m.role !== 'flagged');
}

async function moveTo(view: MailView, threads: ThreadSummary[], b: Mailbox) {
  try {
    await api.move(await view.messageIds(threads), b.id);
    toast(`Moved to ${b.name}`);
    void view.refresh();
  } catch (e) {
    errorToast(e);
  }
}

export function moveMenu(view: MailView, anchor: HTMLElement | { x: number; y: number }, threads = view.targetThreads()) {
  const t = threads[0];
  if (!t) return;
  popupMenu(anchor, [
    ...moveTargets(t.account_id).map((b) => ({ label: b.path, icon: b.is_label ? 'label' : 'folder', run: () => void moveTo(view, threads, b) })),
    '-',
    { label: 'New folder…', icon: 'plus', run: () => void folderDialog(t.account_id).then((f) => f && moveTo(view, threads, f)) },
  ]);
}

/** The palette's commands for the conversations the keys act on: a move to
 *  each folder they aren't all in, and everything with the other person. */
export function paletteCommands(view: MailView): { context?: string; commands: Command[] } {
  const threads = view.targetThreads();
  const t = threads[0];
  if (!t) return { commands: [] };
  const context = threads.length === 1 ? `“${t.subject || '(no subject)'}”` : `${threads.length} conversations`;
  const me = store.state.myEmails;
  const person = threads.length === 1 ? t.participants.find((a) => !me.has(a.email.toLowerCase())) : undefined;
  return {
    context,
    commands: [
      ...moveTargets(t.account_id)
        .filter((b) => !threads.every((x) => x.mailbox_ids.includes(b.id)))
        .map((b) => ({ label: `Move to ${b.path}`, icon: b.is_label ? 'label' : 'folder', run: () => void moveTo(view, threads, b) })),
      { label: 'Move to new folder…', icon: 'plus', run: () => void folderDialog(t.account_id).then((f) => f && moveTo(view, threads, f)) },
      ...(person ? [{ label: `All mail with ${shortName(person)}`, icon: 'user', run: () => openPerson(person) }] : []),
    ],
  };
}

export function moreMenu(view: MailView, anchor: HTMLElement, thread: Thread) {
  const lastMsg = thread.messages[thread.messages.length - 1];
  popupMenu(anchor, [
    { label: 'Mark as unread', icon: 'mail', hint: 'u', run: () => void view.act('unread') },
    ...(view.inSpam()
      ? [
          { label: 'Not spam', icon: 'inbox', hint: '!', run: () => void view.act('not_spam') },
          { label: 'Spam: delete and block sender', icon: 'junk', run: () => void view.act('block') },
        ]
      : [{ label: 'Mark as spam', icon: 'junk', hint: '!', run: () => void view.act('spam') }]),
    { label: 'Forward', icon: 'forward', hint: 'f', run: () => void view.reply('forward') },
    '-',
    { label: 'View source', icon: 'draft', run: () => void viewSource(lastMsg.id) },
  ]);
}
