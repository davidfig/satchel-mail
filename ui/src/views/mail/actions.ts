// Acting on conversations: archive, delete, spam, flags, moves, menus.

import { api } from '../../api';
import { shortName } from '../../lib/format';
import { folderDialog } from '../../lib/folders';
import { openPerson } from '../../lib/person';
import { confirmDialog, errorToast, popupMenu, toast } from '../../lib/ui';
import type { MenuItem } from '../../lib/ui';
import { store } from '../../state';
import type { Mailbox, MessageDetail, Thread, ThreadSummary } from '../../types';
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

export async function act(view: MailView, kind: Action, threads = view.targetThreads()) {
  if (!threads.length) return;
  try {
    const ids = await view.messageIds(threads);
    if (kind === 'archive') await api.archive(ids);
    if (kind === 'trash') {
      const inTrash = threads.every((t) => t.mailbox_ids.every((m) => store.mailbox(m)?.role === 'trash'));
      if (inTrash && !(await confirmDialog('Delete forever?', 'These messages will be permanently deleted.', 'Delete', true))) return;
      await api.trash(ids);
    }
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
    if (kind === 'archive' || kind === 'trash' || kind === 'spam' || kind === 'not_spam' || kind === 'block') {
      const removed = new Set(threads.map((t) => t.thread_id));
      // The store already reflects the change; re-read the window and keep the
      // cursor at the same position, which now holds the next thread.
      const pos = Math.max(0, view.cursor >= 0 ? view.cursor : view.list.indexOf(threads[0].thread_id));
      view.selected.clear();
      view.renderHeader();
      await view.refresh();
      view.cursor = Math.min(pos, view.list.total - 1);
      view.renderRows();
      const next = view.list.at(view.cursor);
      if (next && store.state.selectedThread && removed.has(store.state.selectedThread)) void view.showThread(next.thread_id);
      else if (!next) view.renderEmptyReader();
      const done = { archive: 'Archived', trash: 'Moved to Trash', spam: 'Marked as spam.', not_spam: 'Moved to Inbox.', block: 'Deleted.' }[kind];
      toast(done + note, note ? { ms: 6000 } : {});
    } else {
      void view.refresh();
    }
  } catch (e) {
    errorToast(e);
  }
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
  const how = u.one_click ? `Satchel will tell ${who} to stop sending you this mail.`
    : u.mailto ? `Satchel will send ${who} an unsubscribe email from your account.`
    : `This opens ${who}'s unsubscribe page in your browser.`;
  const ok = inSpam
    ? await confirmDialog('Unsubscribe from mail in Spam?', `Unsubscribing from real spam tells the sender your address works. Only do this for senders you recognize; otherwise leave it in Spam. ${how}`, 'Unsubscribe', true)
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

export function moveMenu(view: MailView, anchor: HTMLElement | { x: number; y: number }, threads = view.targetThreads()) {
  const t = threads[0];
  if (!t) return;
  const boxes = store.state.mailboxes.filter((m) => m.account_id === t.account_id && m.role !== 'all' && m.role !== 'flagged');
  const moveTo = async (b: Mailbox) => {
    try {
      await api.move(await view.messageIds(threads), b.id);
      toast(`Moved to ${b.name}`);
      void view.refresh();
    } catch (e) {
      errorToast(e);
    }
  };
  popupMenu(anchor, [
    ...boxes.map((b) => ({ label: b.path, icon: b.is_label ? 'label' : 'folder', run: () => void moveTo(b) })),
    '-',
    { label: 'New folder…', icon: 'plus', run: () => void folderDialog(t.account_id).then((f) => f && moveTo(f)) },
  ]);
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
