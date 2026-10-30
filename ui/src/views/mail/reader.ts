// The reading pane: the open conversation, its header and actions.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { shortName } from '../../lib/format';
import { personLink } from '../../lib/person';
import { errorToast } from '../../lib/ui';
import { store } from '../../state';
import type { Address, Mailbox, Thread, ThreadSummary } from '../../types';
import { editDraft, openComposer } from './composer';
import { Conversation, mineFor } from './conversation';
import { ReplyBox } from './reply-box';
import { BLOCK_TIP, isJunk } from './actions';
import type { MailView } from './index';

export function renderEmptyReader(view: MailView) {
  ++view.readerSeq;
  view.readerThread = null;
  view.readerLoad = Promise.resolve();
  view.openThread = null;
  view.openSummary = null;
  view.replyBox = null;
  replace(view.reader, h('div.reader-empty', icon('mail', 40), h('p', 'Select a conversation'), h('p.muted.small', 'j/k to move · enter to open · c to compose · / to search')));
}

/** Show a conversation in the reader. Clicking the one already shown does
 *  nothing; `reload` re-fetches it anyway (new messages arrived). */
export function showThread(view: MailView, threadId: string, markRead = true, reload = false): Promise<void> {
  store.set({ selectedThread: threadId });
  view.renderRows();
  if (view.readerThread !== threadId || reload) {
    view.readerThread = threadId;
    view.readerLoad = load(view, threadId, markRead);
  }
  return view.readerLoad;
}

async function load(view: MailView, threadId: string, markRead: boolean) {
  const seq = ++view.readerSeq;
  const gen = view.generation;
  // The old conversation stays up until the new one is ready, so switching
  // doesn't flash an empty pane; only a slow fetch shows "Loading…". Until
  // then its buttons and keys act on the new selection (`targetThreads`),
  // so the reply box and thread they'd use go now.
  view.openThread = null;
  view.replyBox = null;
  const slow = window.setTimeout(() => {
    if (seq === view.readerSeq) replace(view.reader, h('div.reader-loading', 'Loading…'));
  }, LOADING_DELAY_MS);
  try {
    const thread = await api.getThread(threadId);
    clearTimeout(slow);
    if (seq !== view.readerSeq) return;
    if (gen !== view.generation || store.state.selectedThread !== threadId) {
      view.readerThread = null;
      return;
    }
    view.openThread = thread;
    // Opened from outside the loaded rows (a notification, Review, or
    // scrolled away): build the summary so the header's actions still work.
    view.openSummary = view.list.find(threadId) ?? summaryOf(thread);
    await renderThread(view, thread, seq);
    if (seq !== view.readerSeq) return;
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
    clearTimeout(slow);
    if (seq !== view.readerSeq) return;
    errorToast(e);
    view.renderEmptyReader();
  }
}

/** How long a fetch may take before the reader says it's loading. */
const LOADING_DELAY_MS = 250;
/** The longest we keep the old conversation up waiting for message frames. */
const FRAME_WAIT_MS = 400;

function summaryOf(thread: Thread): ThreadSummary {
  const ms = thread.messages;
  const last = ms[ms.length - 1];
  const people = new Map<string, Address>();
  for (const m of ms) for (const a of [m.from, ...m.to]) if (a) people.set(a.email.toLowerCase(), a);
  return {
    thread_id: thread.thread_id,
    account_id: thread.account_id,
    subject: thread.subject,
    participants: [...people.values()],
    last_date: last?.date ?? '',
    message_count: ms.length,
    unread_count: ms.filter((m) => !m.flags.seen).length,
    flagged: ms.some((m) => m.flags.flagged),
    has_attachments: ms.some((m) => m.has_attachments),
    snippet: last?.snippet ?? '',
    mailbox_ids: [...new Set(ms.flatMap((m) => m.mailbox_ids))],
    latest_message_id: last?.id ?? '',
    favorite: ms.some((m) => !!m.from && store.state.favorites.has(m.from.email.toLowerCase())),
  };
}

/** Builds the conversation as a page and swaps it in for what the reader
 *  shows (see `present`); resolves once it's on screen, or abandoned. */
async function renderThread(view: MailView, thread: Thread, seq: number) {
  const labels = [...new Set(thread.messages.flatMap((m) => m.mailbox_ids))].map((id) => store.mailbox(id)).filter((m): m is Mailbox => !!m && m.role !== 'inbox');
  const mine = mineFor(thread);
  const people = new Map<string, Address>();
  for (const m of thread.messages) for (const a of [m.from, ...m.to, ...m.cc]) if (a && !mine(a)) people.set(a.email.toLowerCase(), a);
  const spam = isJunk(thread.messages.flatMap((m) => m.mailbox_ids));
  const convo = new Conversation(thread, (mode, id) => void view.reply(mode, id), { spam, onUnsubscribe: (m) => void view.unsubscribe(m, spam) });
  const blocked = thread.messages.find((m) => m.sender_verdict === 'spam' && m.from);
  const replyBox = new ReplyBox(thread);
  const page = h('div.reader-page',
    h('header.reader-head',
      h('div.reader-title',
        h('h2.reader-subject', thread.subject || '(no subject)'),
        h('div.muted.small.reader-people', [...people.values()].map((a) => [personLink(a, shortName(a)), ', ']), 'you')),
      h('div.reader-actions',
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
    replyBox.root,
  );
  if (!(await present(view, page, seq))) return;
  view.replyBox = replyBox;
  convo.settle(view.reader);
}

/** Puts `page` in the reader in one step. HTML messages render in iframes,
 *  which load blank and then grow to fit, so the page is laid out hidden
 *  behind the current one until they've loaded (or `FRAME_WAIT_MS` passes),
 *  then revealed in place: moving an iframe would reload it. False if a newer
 *  conversation was asked for meanwhile. */
async function present(view: MailView, page: HTMLElement, seq: number): Promise<boolean> {
  const old = [...view.reader.children];
  const frames = [...page.querySelectorAll('iframe')];
  if (!old.length || !frames.length) {
    replace(view.reader, page);
    return true;
  }
  page.classList.add('staging');
  view.reader.append(page);
  await Promise.race([
    Promise.all(frames.map((f) => new Promise((r) => f.addEventListener('load', r, { once: true })))),
    new Promise((r) => setTimeout(r, FRAME_WAIT_MS)),
  ]);
  if (seq !== view.readerSeq) {
    page.remove();
    return false;
  }
  for (const el of old) el.remove();
  page.classList.remove('staging');
  return true;
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
