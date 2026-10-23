// Mail: thread list (virtualized) + reading pane.

import { api } from '../api';
import { debounce, h, icon, replace } from '../lib/dom';
import { formatAddress, listDate, participantsLabel, shortName } from '../lib/format';
import { bind, confirmDialog, errorToast, popupMenu, toast } from '../lib/ui';
import { store } from '../state';
import type { Address, Mailbox, Thread, ThreadQuery, ThreadSummary } from '../types';
import { openComposer } from './composer';
import { Conversation, ReplyBox, mineFor, viewSource } from './conversation';
import { ThreadWindow } from './thread-window';

const ROW_H = 74;
// Browsers cap element heights (~17M px in WebKit); beyond this the scrollbar is scaled.
const MAX_SCROLL_PX = 8_000_000;

export class MailView {
  root = h('div.mail-view');
  private listPane = h('section.list-pane', { 'aria-label': 'Conversations' });
  private reader = h('section.reader', { 'aria-label': 'Message' });
  private scroller = h('div.list-scroll', { tabindex: 0, role: 'listbox', 'aria-label': 'Conversations' });
  private spacer = h('div.list-spacer');
  private rows = h('div.list-rows');
  private header = h('header.list-head');
  private searchInput = h('input.search', { type: 'search', placeholder: 'Search mail', 'aria-label': 'Search mail' });
  private list = new ThreadWindow(() => this.query(), () => this.renderRows(), (e) => errorToast(e));
  private cursor = -1;
  /** Selected threads by id (kept even if their page is evicted). */
  private selected = new Map<string, ThreadSummary>();
  private openThread: Thread | null = null;
  private replyBox: ReplyBox | null = null;
  private generation = 0;

  constructor() {
    this.spacer.appendChild(this.rows);
    this.scroller.appendChild(this.spacer);
    this.listPane.append(this.header, this.scroller);
    this.root.append(this.listPane, this.reader);
    this.scroller.addEventListener('scroll', () => this.renderRows());
    new ResizeObserver(() => this.renderRows()).observe(this.scroller);
    this.searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const q = this.searchInput.value.trim();
        if (q) store.set({ source: { kind: 'search', query: q, label: `Search: ${q}` }, selectedThread: null });
        else store.set({ source: { kind: 'role', role: 'inbox', label: 'Inbox' } });
        this.scroller.focus();
      } else if (e.key === 'Escape') {
        this.searchInput.blur();
        this.scroller.focus();
      }
    });
    store.subscribe((_, changed) => {
      if (changed.has('source')) void this.reload();
    });
    const refresh = debounce(() => void this.refresh(), 300);
    store.onEvent((e) => {
      if (e.event === 'messages_changed' || e.event === 'data_changed') refresh();
    });
    this.renderHeader();
    this.renderEmptyReader();
    this.bindKeys();
    void this.reload();
  }

  show() {
    const sel = store.state.selectedThread;
    if (sel && this.openThread?.thread_id !== sel) void this.showThread(sel, false);
    this.scroller.focus({ preventScroll: true });
  }

  private query(): ThreadQuery {
    const s = store.state.source;
    const q: ThreadQuery = {};
    if (s.kind === 'role') q.role = s.role;
    else if (s.kind === 'mailbox') q.mailbox_id = s.mailbox_id;
    else q.search = s.query;
    return q;
  }

  private renderHeader() {
    const s = store.state.source;
    if (s.kind === 'search' && this.searchInput.value !== s.query) this.searchInput.value = s.query;
    if (s.kind !== 'search' && document.activeElement !== this.searchInput) this.searchInput.value = '';
    const n = this.selected.size;
    replace(
      this.header,
      h('div.list-title',
        h('h1', s.label),
        h('span.spacer'),
        h('button.icon-btn', { title: 'Refresh (sync now)', 'aria-label': 'Sync now', on: { click: () => void api.syncNow(null).then(() => toast('Syncing…')) } }, icon('refresh')),
        h('button.icon-btn', { title: 'Compose (c)', 'aria-label': 'Compose', on: { click: () => openComposer({}) } }, icon('compose'))),
      this.searchInput,
      n > 0 &&
        h('div.bulk-bar',
          h('span', `${n} selected`),
          h('button', { on: { click: () => void this.act('archive') } }, icon('archive', 16), 'Archive'),
          h('button', { on: { click: () => void this.act('read') } }, icon('mailOpen', 16), 'Read'),
          h('button', { on: { click: () => void this.act('trash') } }, icon('trash', 16), 'Delete'),
          h('button.link', { on: { click: () => { this.selected.clear(); this.renderHeader(); this.renderRows(); } } }, 'Clear')),
    );
  }

  async reload() {
    ++this.generation;
    this.cursor = -1;
    this.selected.clear();
    this.scroller.scrollTop = 0;
    this.renderHeader();
    await this.list.reset();
    const sel = store.state.selectedThread;
    if (sel) {
      this.cursor = this.list.indexOf(sel);
      this.renderRows();
    }
  }

  // ---------------------------------------------------------------- geometry
  // Rows live in a logical space of total * ROW_H pixels. When that exceeds what
  // browsers can render, the scroll range is compressed by `scale`.

  private scale(): number {
    const logical = this.list.total * ROW_H;
    const ch = this.scroller.clientHeight;
    if (logical <= MAX_SCROLL_PX) return 1;
    return (logical - ch) / Math.max(MAX_SCROLL_PX - ch, 1);
  }

  private logicalTop(): number {
    return this.scroller.scrollTop * this.scale();
  }

  private visibleRange(pad = 5): [number, number] {
    const top = this.logicalTop();
    const first = Math.max(0, Math.floor(top / ROW_H) - pad);
    const last = Math.min(this.list.total, Math.ceil((top + this.scroller.clientHeight) / ROW_H) + pad);
    return [first, last];
  }

  private scrollToIndex(i: number, offset = 0) {
    this.scroller.scrollTop = (i * ROW_H + offset) / this.scale();
  }

  /** Re-fetch the visible window in place (after sync or local changes), keeping
   *  the top visible thread where it is. */
  async refresh() {
    const gen = this.generation;
    const top = this.logicalTop();
    const anchorIdx = Math.floor(top / ROW_H);
    const anchorId = this.scroller.scrollTop > 0 ? this.list.at(anchorIdx)?.thread_id : undefined;
    const within = top - anchorIdx * ROW_H;
    const cursorId = this.list.at(this.cursor)?.thread_id;
    const [first, last] = this.visibleRange(20);
    await this.list.refresh(first, last);
    if (gen !== this.generation) return;
    if (anchorId) {
      const now = this.list.indexOf(anchorId);
      if (now >= 0 && now !== anchorIdx) this.scrollToIndex(now, within);
    }
    if (cursorId) {
      const now = this.list.indexOf(cursorId);
      this.cursor = now >= 0 ? now : Math.min(this.cursor, this.list.total - 1);
    }
    this.renderRows();
    if (this.openThread) {
      const still = this.list.find(this.openThread.thread_id);
      if (still && still.message_count !== this.openThread.messages.length) void this.showThread(still.thread_id, false);
    }
  }

  private renderRows() {
    const total = this.list.total;
    this.spacer.style.height = `${Math.max(Math.min(total * ROW_H, MAX_SCROLL_PX), 1)}px`;
    if (!total) {
      replace(this.rows, h('div.empty-list', this.list.loaded ? emptyMessage() : 'Loading…'));
      this.rows.style.transform = '';
      return;
    }
    const [first, last] = this.visibleRange();
    this.list.ensure(first, last);
    // Position the rendered slice relative to the (possibly compressed) scroll offset.
    this.rows.style.transform = `translateY(${this.scroller.scrollTop + (first * ROW_H - this.logicalTop())}px)`;
    const me = store.state.myEmails;
    const out: HTMLElement[] = [];
    for (let idx = first; idx < last; idx++) {
      const t = this.list.at(idx);
      if (!t) {
        out.push(h('div.thread-row.placeholder', { style: { height: `${ROW_H}px` }, 'aria-hidden': 'true' }, h('div.thread-main', h('div.ph.w40'), h('div.ph.w70'), h('div.ph.w90'))));
        continue;
      }
      const active = t.thread_id === store.state.selectedThread;
      out.push(
        h(
          `div.thread-row${t.unread_count ? '.unread' : ''}${active ? '.active' : ''}${idx === this.cursor ? '.cursor' : ''}${this.selected.has(t.thread_id) ? '.selected' : ''}`,
          {
            role: 'option',
            'aria-selected': active ? 'true' : 'false',
            style: { height: `${ROW_H}px` },
            on: {
              click: (e: MouseEvent) => {
                if (e.metaKey || e.ctrlKey || e.shiftKey) {
                  this.toggleSelect(t);
                  return;
                }
                this.cursor = idx;
                void this.showThread(t.thread_id);
              },
            },
          },
          h('span.unread-dot', { 'aria-label': t.unread_count ? 'Unread' : undefined }),
          store.state.accounts.length > 1 && h('span.acct-bar', { style: { background: store.accountColor(t.account_id) }, title: store.state.accounts.find((a) => a.id === t.account_id)?.email }),
          h('div.thread-main',
            h('div.thread-line1',
              h('span.who', participantsLabel(t.participants, me)),
              t.message_count > 1 && h('span.count', String(t.message_count)),
              h('span.spacer'),
              t.has_attachments && icon('paperclip', 14),
              h('time', { datetime: t.last_date }, listDate(t.last_date))),
            h('div.thread-subject', t.subject || '(no subject)'),
            h('div.thread-snippet', t.snippet)),
          h('button.star', {
            'aria-label': t.flagged ? 'Unflag' : 'Flag',
            'aria-pressed': t.flagged ? 'true' : 'false',
            on: { click: (e: MouseEvent) => { e.stopPropagation(); void this.flagThread(t); } },
          }, icon('star', 16)),
        ),
      );
    }
    replace(this.rows, out);
  }

  private toggleSelect(t: ThreadSummary) {
    if (this.selected.has(t.thread_id)) this.selected.delete(t.thread_id);
    else this.selected.set(t.thread_id, t);
    this.renderHeader();
    this.renderRows();
  }

  private targetThreads(): ThreadSummary[] {
    if (this.selected.size) return [...this.selected.values()];
    const sel = store.state.selectedThread;
    const t = (sel ? this.list.find(sel) : undefined) ?? this.list.at(this.cursor);
    return t ? [t] : [];
  }

  private async messageIds(threads: ThreadSummary[]): Promise<string[]> {
    if (threads.length === 1 && this.openThread?.thread_id === threads[0].thread_id) return this.openThread.messages.map((m) => m.id);
    const out: string[] = [];
    for (const t of threads) {
      const full = await api.getThread(t.thread_id);
      out.push(...full.messages.map((m) => m.id));
    }
    return out;
  }

  private async act(kind: 'archive' | 'trash' | 'read' | 'unread' | 'spam') {
    const threads = this.targetThreads();
    if (!threads.length) return;
    try {
      const ids = await this.messageIds(threads);
      if (kind === 'archive') await api.archive(ids);
      if (kind === 'trash') {
        const inTrash = threads.every((t) => t.mailbox_ids.every((m) => store.mailbox(m)?.role === 'trash'));
        if (inTrash && !(await confirmDialog('Delete forever?', 'These messages will be permanently deleted.', 'Delete', true))) return;
        await api.trash(ids);
      }
      if (kind === 'read') await api.setFlags(ids, true, null);
      if (kind === 'unread') await api.setFlags(ids, false, null);
      if (kind === 'spam') {
        const junk = store.state.mailboxes.find((m) => m.account_id === threads[0].account_id && m.role === 'junk');
        if (!junk) return toast('No Junk folder for this account', { kind: 'error' });
        await api.move(ids, junk.id);
      }
      if (kind === 'archive' || kind === 'trash' || kind === 'spam') {
        const removed = new Set(threads.map((t) => t.thread_id));
        // The store already reflects the change; re-read the window and keep the
        // cursor at the same position, which now holds the next thread.
        const pos = Math.max(0, this.cursor >= 0 ? this.cursor : this.list.indexOf(threads[0].thread_id));
        this.selected.clear();
        this.renderHeader();
        await this.refresh();
        this.cursor = Math.min(pos, this.list.total - 1);
        this.renderRows();
        const next = this.list.at(this.cursor);
        if (next && store.state.selectedThread && removed.has(store.state.selectedThread)) void this.showThread(next.thread_id);
        else if (!next) this.renderEmptyReader();
        toast(kind === 'archive' ? 'Archived' : kind === 'spam' ? 'Marked as spam' : 'Moved to Trash');
      } else {
        void this.refresh();
      }
    } catch (e) {
      errorToast(e);
    }
  }

  private async flagThread(t: ThreadSummary) {
    try {
      const full = await api.getThread(t.thread_id);
      const target = full.messages[full.messages.length - 1];
      await api.setFlags(t.flagged ? full.messages.map((m) => m.id) : [target.id], null, !t.flagged);
      t.flagged = !t.flagged;
      this.renderRows();
    } catch (e) {
      errorToast(e);
    }
  }

  private renderEmptyReader() {
    this.openThread = null;
    this.replyBox = null;
    replace(this.reader, h('div.reader-empty', icon('mail', 40), h('p', 'Select a conversation'), h('p.muted.small', 'j/k to move · enter to open · c to compose · / to search')));
  }

  async showThread(threadId: string, markRead = true) {
    store.set({ selectedThread: threadId });
    this.renderRows();
    const gen = this.generation;
    replace(this.reader, h('div.reader-loading', 'Loading…'));
    try {
      const thread = await api.getThread(threadId);
      if (gen !== this.generation || store.state.selectedThread !== threadId) return;
      this.openThread = thread;
      this.renderThread(thread);
      const unread = thread.messages.filter((m) => !m.flags.seen).map((m) => m.id);
      if (markRead && unread.length) {
        await api.setFlags(unread, true, null);
        const t = this.list.find(threadId);
        if (t) {
          t.unread_count = 0;
          this.renderRows();
        }
      }
    } catch (e) {
      errorToast(e);
      this.renderEmptyReader();
    }
  }

  private renderThread(thread: Thread) {
    const labels = [...new Set(thread.messages.flatMap((m) => m.mailbox_ids))].map((id) => store.mailbox(id)).filter((m): m is Mailbox => !!m && m.role !== 'inbox');
    const mine = mineFor(thread);
    const people = new Map<string, Address>();
    for (const m of thread.messages) for (const a of [m.from, ...m.to, ...m.cc]) if (a && !mine(a)) people.set(a.email.toLowerCase(), a);
    const convo = new Conversation(thread, (mode, id) => void this.reply(mode, id));
    this.replyBox = new ReplyBox(thread);
    replace(
      this.reader,
      h('header.reader-head',
        h('div.reader-title',
          h('h2.reader-subject', thread.subject || '(no subject)'),
          h('div.muted.small', [...[...people.values()].map(shortName), 'you'].join(', '))),
        h('div.reader-actions',
          h('button.icon-btn', { title: 'Archive (e)', 'aria-label': 'Archive', on: { click: () => void this.act('archive') } }, icon('archive')),
          h('button.icon-btn', { title: 'Delete (#)', 'aria-label': 'Delete', on: { click: () => void this.act('trash') } }, icon('trash')),
          h('button.icon-btn', { title: 'Mark unread (u)', 'aria-label': 'Mark unread', on: { click: () => void this.act('unread') } }, icon('mail')),
          h('button.icon-btn', { title: 'Move to… (v)', 'aria-label': 'Move', on: { click: (e: MouseEvent) => this.moveMenu(e.currentTarget as HTMLElement) } }, icon('folder')),
          h('button.icon-btn', { title: 'More', 'aria-label': 'More actions', on: { click: (e: MouseEvent) => this.moreMenu(e.currentTarget as HTMLElement, thread) } }, icon('more'))),
        labels.length > 0 && h('div.labels', labels.map((m) => h('span.label-chip', m.name)))),
      convo.root,
      this.replyBox.root,
    );
    convo.settle(this.reader);
  }

  private async reply(mode: 'reply' | 'reply_all' | 'forward', messageId?: string) {
    const t = this.openThread;
    if (!t) return;
    const target = messageId ?? [...t.messages].reverse().find((m) => !store.state.myEmails.has(m.from?.email.toLowerCase() ?? ''))?.id ?? t.messages[t.messages.length - 1].id;
    try {
      const d = await api.composeReply(target, mode);
      openComposer(d);
    } catch (e) {
      errorToast(e);
    }
  }

  private moveMenu(anchor: HTMLElement) {
    const t = this.targetThreads()[0];
    if (!t) return;
    const boxes = store.state.mailboxes.filter((m) => m.account_id === t.account_id && m.role !== 'all' && m.role !== 'flagged');
    popupMenu(anchor, boxes.map((b) => ({
      label: b.path,
      icon: b.is_label ? 'label' : 'folder',
      run: async () => {
        try {
          await api.move(await this.messageIds(this.targetThreads()), b.id);
          toast(`Moved to ${b.name}`);
          void this.refresh();
        } catch (e) {
          errorToast(e);
        }
      },
    })));
  }

  private moreMenu(anchor: HTMLElement, thread: Thread) {
    const lastMsg = thread.messages[thread.messages.length - 1];
    popupMenu(anchor, [
      { label: 'Mark as unread', icon: 'mail', hint: 'u', run: () => void this.act('unread') },
      { label: 'Mark as spam', icon: 'junk', hint: '!', run: () => void this.act('spam') },
      { label: 'Forward', icon: 'forward', hint: 'f', run: () => void this.reply('forward') },
      '-',
      { label: 'View source', icon: 'draft', run: () => void viewSource(lastMsg.id) },
    ]);
  }

  private async move(delta: number) {
    if (!this.list.total) return;
    this.cursor = Math.max(0, Math.min(this.list.total - 1, this.cursor + delta));
    const y = this.cursor * ROW_H;
    const top = this.logicalTop();
    if (y < top) this.scrollToIndex(this.cursor);
    else if (y + ROW_H > top + this.scroller.clientHeight) this.scrollToIndex(this.cursor, ROW_H - this.scroller.clientHeight);
    this.renderRows();
    if (store.state.selectedThread) {
      const t = await this.list.get(this.cursor);
      if (t) void this.showThread(t.thread_id);
    }
  }

  private async openAtCursor() {
    const t = await this.list.get(this.cursor);
    if (t) void this.showThread(t.thread_id);
  }

  private bindKeys() {
    const s = 'mail';
    bind('j', 'Next conversation', () => void this.move(1), s);
    bind('ArrowDown', 'Next conversation', () => void this.move(1), s);
    bind('k', 'Previous conversation', () => void this.move(-1), s);
    bind('ArrowUp', 'Previous conversation', () => void this.move(-1), s);
    bind('Enter', 'Open conversation', () => void this.openAtCursor(), s);
    bind('o', 'Open conversation', () => void this.openAtCursor(), s);
    bind('x', 'Select conversation', () => { const t = this.list.at(this.cursor); if (t) this.toggleSelect(t); }, s);
    bind('e', 'Archive', () => void this.act('archive'), s);
    bind('#', 'Delete', () => void this.act('trash'), s);
    bind('Backspace', 'Delete', () => void this.act('trash'), s);
    bind('u', 'Mark unread', () => void this.act('unread'), s);
    bind('shift+i', 'Mark read', () => void this.act('read'), s);
    bind('!', 'Mark as spam', () => void this.act('spam'), s);
    bind('s', 'Flag / unflag', () => { const t = this.targetThreads()[0]; if (t) void this.flagThread(t); }, s);
    bind('r', 'Reply', () => this.replyBox?.focus('reply'), s);
    bind('a', 'Reply all', () => this.replyBox?.focus('reply_all'), s);
    bind('f', 'Forward', () => void this.reply('forward'), s);
    bind('v', 'Move to…', () => { const b = this.reader.querySelector<HTMLElement>('[aria-label="Move"]'); if (b) this.moveMenu(b); }, s);
    bind('/', 'Search', () => { this.searchInput.focus(); this.searchInput.select(); }, s);
    bind('Escape', 'Close conversation', () => { store.set({ selectedThread: null }); this.renderEmptyReader(); this.renderRows(); this.scroller.focus(); }, s);
  }
}

function emptyMessage(): string {
  const s = store.state.source;
  if (!store.state.accounts.length) return 'No accounts yet — add one in Settings.';
  if (s.kind === 'search') return 'No matching conversations.';
  return s.kind === 'role' && s.role === 'inbox' ? 'Inbox zero. 🎉' : 'Nothing here.';
}

export function addressList(list: { email: string; name?: string | null }[]) {
  return list.map(formatAddress).join(', ');
}
