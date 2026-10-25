// The reading pane: the open conversation, its header and actions.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { shortName } from '../../lib/format';
import { personLink } from '../../lib/person';
import { errorToast } from '../../lib/ui';
import { store } from '../../state';
import type { Address, Mailbox, Thread } from '../../types';
import { editDraft, openComposer } from './composer';
import { Conversation, mineFor } from './conversation';
import { ReplyBox } from './reply-box';
import { BLOCK_TIP, isJunk } from './actions';
import type { MailView } from './index';

export function renderEmptyReader(view: MailView) {
  view.openThread = null;
  view.openSummary = null;
  view.replyBox = null;
  replace(view.reader, h('div.reader-empty', icon('mail', 40), h('p', 'Select a conversation'), h('p.muted.small', 'j/k to move · enter to open · c to compose · / to search')));
}

export async function showThread(view: MailView, threadId: string, markRead = true) {
  store.set({ selectedThread: threadId });
  view.renderRows();
  const gen = view.generation;
  replace(view.reader, h('div.reader-loading', 'Loading…'));
  try {
    const thread = await api.getThread(threadId);
    if (gen !== view.generation || store.state.selectedThread !== threadId) return;
    view.openThread = thread;
    view.openSummary = view.list.find(threadId) ?? null;
    view.renderThread(thread);
    // Opening a draft from Drafts means finishing it.
    const src = store.state.source;
    const draft = [...thread.messages].reverse().find((m) => m.flags.draft);
    if (markRead && draft && src.kind === 'role' && src.role === 'drafts') void editDraft(draft.id);
    const unread = thread.messages.filter((m) => !m.flags.seen).map((m) => m.id);
    if (markRead && unread.length) {
      await api.setFlags(unread, true, null);
      const t = view.list.find(threadId);
      if (t) {
        t.unread_count = 0;
        view.renderRows();
      }
    }
  } catch (e) {
    errorToast(e);
    view.renderEmptyReader();
  }
}

export function renderThread(view: MailView, thread: Thread) {
  const labels = [...new Set(thread.messages.flatMap((m) => m.mailbox_ids))].map((id) => store.mailbox(id)).filter((m): m is Mailbox => !!m && m.role !== 'inbox');
  const mine = mineFor(thread);
  const people = new Map<string, Address>();
  for (const m of thread.messages) for (const a of [m.from, ...m.to, ...m.cc]) if (a && !mine(a)) people.set(a.email.toLowerCase(), a);
  const spam = isJunk(thread.messages.flatMap((m) => m.mailbox_ids));
  const convo = new Conversation(thread, (mode, id) => void view.reply(mode, id), { spam });
  view.replyBox = new ReplyBox(thread);
  const unsub = [...thread.messages].reverse().find((m) => m.unsubscribe);
  const blocked = thread.messages.find((m) => m.sender_verdict === 'spam' && m.from);
  replace(
    view.reader,
    h('header.reader-head',
      h('div.reader-title',
        h('h2.reader-subject', thread.subject || '(no subject)'),
        h('div.muted.small.reader-people', [...people.values()].map((a) => [personLink(a, shortName(a)), ', ']), 'you')),
      h('div.reader-actions',
        unsub && h('button.unsubscribe', { title: 'Ask the sender to stop sending you this mail', on: { click: () => void view.unsubscribe(unsub, spam) } }, 'Unsubscribe'),
        spam
          ? [
              h('button.icon-btn', { title: 'Not spam (!)', 'aria-label': 'Not spam', on: { click: () => void view.act('not_spam') } }, icon('inbox')),
              h('button.icon-btn', { title: BLOCK_TIP, 'aria-label': 'Spam', on: { click: () => void view.act('block') } }, icon('junk')),
            ]
          : h('button.icon-btn', { title: 'Mark as spam (!)', 'aria-label': 'Mark as spam', on: { click: () => void view.act('spam') } }, icon('junk')),
        h('button.icon-btn', { title: 'Archive (e)', 'aria-label': 'Archive', on: { click: () => void view.act('archive') } }, icon('archive')),
        h('button.icon-btn', { title: 'Delete (#)', 'aria-label': 'Delete', on: { click: () => void view.act('trash') } }, icon('trash')),
        h('button.icon-btn', { title: 'Mark unread (u)', 'aria-label': 'Mark unread', on: { click: () => void view.act('unread') } }, icon('mail')),
        h('button.icon-btn', { title: 'Move to… (v)', 'aria-label': 'Move', on: { click: (e: MouseEvent) => view.moveMenu(e.currentTarget as HTMLElement) } }, icon('folder')),
        h('button.icon-btn', { title: 'More', 'aria-label': 'More actions', on: { click: (e: MouseEvent) => view.moreMenu(e.currentTarget as HTMLElement, thread) } }, icon('more'))),
      labels.length > 0 && h('div.labels', labels.map((m) => h('span.label-chip', m.name)))),
    spam &&
      h('div.banner.warn.spam-banner',
        icon('junk', 16),
        h('span.grow', blocked
          ? `You marked ${blocked.from!.email} as spam, so their mail always comes here. Links ask before opening and invitations can’t be answered.`
          : 'This conversation is in Spam. Links ask before opening and invitations can’t be answered.'),
        h('div.banner-actions',
          h('button', { title: 'Move to Inbox and keep this sender out of Spam (!)', on: { click: () => void view.act('not_spam') } }, 'Not spam'),
          h('button', { title: BLOCK_TIP, on: { click: () => void view.act('block') } }, 'Spam'))),
    convo.root,
    view.replyBox.root,
  );
  convo.settle(view.reader);
}

export async function reply(view: MailView, mode: 'reply' | 'reply_all' | 'forward', messageId?: string) {
  const t = view.openThread;
  if (!t) return;
  const target = messageId ?? [...t.messages].reverse().find((m) => !store.state.myEmails.has(m.from?.email.toLowerCase() ?? ''))?.id ?? t.messages[t.messages.length - 1].id;
  try {
    const d = await api.composeReply(target, mode);
    openComposer(d);
  } catch (e) {
    errorToast(e);
  }
}
