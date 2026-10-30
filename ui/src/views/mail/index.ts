// Mail: thread list (virtualized) + reading pane. MailView holds the state and
// coordinates; the list, reader, actions and person header live beside it.

import { api } from '../../api';
import { debounce, h, icon, patch } from '../../lib/dom';
import { emptiable, emptyFolder } from '../../lib/folders';
import { formatAddress } from '../../lib/format';
import { bind, commandSource, contextMenu, errorToast, toast } from '../../lib/ui';
import type { MenuItem } from '../../lib/ui';
import { store } from '../../state';
import type { MailSource } from '../../state';
import type { MessageDetail, PersonSummary, Thread, ThreadQuery, ThreadSummary } from '../../types';
import { openComposer } from './composer';
import type { ReplyBox } from './reply-box';
import { ThreadWindow } from './thread-window';
import * as actions from './actions';
import { type Action, BLOCK_TIP } from './actions';
import * as list from './list';
import { ROW_H } from './list';
import * as person from './person';
import { personQuery } from './person';
import * as reader from './reader';

export class MailView {
  root = h('div.mail-view');
  listPane = h('section.list-pane', { 'aria-label': 'Conversations' });
  reader = h('section.reader', { 'aria-label': 'Message' });
  scroller = h('div.list-scroll', { tabindex: 0, role: 'listbox', 'aria-label': 'Conversations' });
  spacer = h('div.list-spacer');
  rows = h('div.list-rows');
  header = h('header.list-head');
  searchInput = h('input.search', { type: 'search', placeholder: 'Search mail', 'aria-label': 'Search mail' });
  list = new ThreadWindow(() => this.query(), () => this.renderRows(), (e) => errorToast(e));
  cursor = -1;
  /** Selected threads by id (kept even if their page is evicted). */
  selected = new Map<string, ThreadSummary>();
  openThread: Thread | null = null;
  /** The list row of the open conversation, kept when a refresh drops it
   *  from the list (unflagged in Flagged, read under `is:unread`). */
  openSummary: ThreadSummary | null = null;
  replyBox: ReplyBox | null = null;
  /** The conversation the reader shows or is loading. */
  readerThread: string | null = null;
  /** Bumped by every reader change, so a slower earlier load backs off. */
  readerSeq = 0;
  /** Settles when the reader has shown `readerThread`. */
  readerLoad: Promise<void> = Promise.resolve();
  generation = 0;

  constructor() {
    this.spacer.appendChild(this.rows);
    this.scroller.appendChild(this.spacer);
    this.listPane.append(this.header, this.scroller);
    this.root.append(this.listPane, this.reader);
    this.scroller.addEventListener('scroll', () => this.renderRows());
    contextMenu(this.rows, (e) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>('.thread-row[data-idx]');
      const t = row && this.list.at(Number(row.dataset.idx));
      if (!t) return null;
      this.cursor = Number(row.dataset.idx);
      this.renderRows();
      return this.rowMenu(t, { x: e.clientX, y: e.clientY });
    });
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
    let images = imageSettings();
    store.subscribe((_, changed) => {
      // The open thread was drawn under the old remote image settings.
      if (changed.has('settings') && imageSettings() !== images) {
        images = imageSettings();
        if (this.readerThread) void this.showThread(this.readerThread, false, true);
      }
      if (changed.has('source')) void this.reload();
      // A folder renamed here or elsewhere keeps its title current, and
      // Empty Spam/Trash knows when there's nothing left.
      else if (changed.has('mailboxes') && (store.state.source.kind === 'mailbox' || emptiable(store.state.source))) this.renderHeader();
      else if (changed.has('favorites') && store.state.source.kind === 'person') this.renderHeader();
    });
    const refresh = debounce(() => void this.refresh(), 300);
    store.onEvent((e) => {
      // Rows say whether a favorite wrote them, and `is:favorite` lists them.
      if (e.event === 'messages_changed' || e.event === 'data_changed' || e.event === 'favorites_changed') refresh();
    });
    this.renderHeader();
    this.renderEmptyReader();
    this.bindKeys();
    void this.reload();
  }

  show() {
    const sel = store.state.selectedThread;
    if (sel) void this.showThread(sel, false);
    this.scroller.focus({ preventScroll: true });
  }

  private query(): ThreadQuery {
    const s = store.state.source;
    const q: ThreadQuery = {};
    if (s.kind === 'role') q.role = s.role;
    else if (s.kind === 'mailbox') q.mailbox_id = s.mailbox_id;
    else if (s.kind === 'person') q.search = personQuery(s.emails, s.filter);
    else q.search = s.query;
    return q;
  }

  renderHeader() {
    const s = store.state.source;
    if (s.kind === 'person') this.loadPerson(s.emails);
    if (s.kind === 'search' && this.searchInput.value !== s.query) this.searchInput.value = s.query;
    if (s.kind !== 'search' && document.activeElement !== this.searchInput) this.searchInput.value = '';
    const n = this.selected.size;
    // Patched: this redraws on sync ticks while Spam, Trash or a folder is
    // open, and the search box keeps its focus.
    patch(
      this.header,
      h('div.list-title',
        s.kind === 'person' && h('button.icon-btn', { title: `Back to ${s.back?.label ?? 'Inbox'}`, 'aria-label': 'Back', on: { click: () => store.set({ source: s.back ?? { kind: 'role', role: 'inbox', label: 'Inbox' } }) } }, icon('chevronLeft')),
        h('h1', (s.kind === 'person' && this.person?.summary?.name) || (s.kind === 'mailbox' && store.mailbox(s.mailbox_id)?.path) || s.label),
        h('span.spacer'),
        this.emptyButton(s),
        h('button.icon-btn', { title: 'Refresh (sync now)', 'aria-label': 'Sync now', on: { click: () => void api.syncNow(null).then(() => toast('Syncing…', { quiet: true })) } }, icon('refresh')),
        h('button.icon-btn', { title: 'Compose (c)', 'aria-label': 'Compose', on: { click: () => openComposer({}) } }, icon('compose'))),
      s.kind === 'person' ? this.personCard(s) : this.searchInput,
      n > 0 &&
        h('div.bulk-bar',
          h('span', `${n} selected`),
          h('button', { title: 'Archive selected (e)', on: { click: () => void this.act('archive') } }, icon('archive', 16), 'Archive'),
          h('button', { title: 'Mark selected as read (Shift+I)', on: { click: () => void this.act('read') } }, icon('mailOpen', 16), 'Read'),
          h('button', { title: 'Move selected to Trash (#)', on: { click: () => void this.act('trash') } }, icon('trash', 16), 'Delete'),
          this.inSpam()
            ? [
                h('button', { title: 'Not spam: move to Inbox and keep these senders out of Spam (!)', on: { click: () => void this.act('not_spam') } }, icon('inbox', 16), 'Not spam'),
                h('button', { title: BLOCK_TIP, on: { click: () => void this.act('block') } }, icon('junk', 16), 'Spam'),
              ]
            : h('button', { title: 'Mark as spam: these senders always go to Spam (!)', on: { click: () => void this.act('spam') } }, icon('junk', 16), 'Spam'),
          h('button.link', { title: 'Clear the selection', on: { click: () => { this.selected.clear(); this.renderHeader(); this.renderRows(); } } }, 'Clear')),
    );
  }

  /** "Empty Spam" / "Empty Trash" while one of those is open. */
  private emptyButton(s: MailSource) {
    const e = emptiable(s);
    return e && h('button.icon-btn', { title: `Empty ${e.label}: permanently delete everything in it`, 'aria-label': `Empty ${e.label}`, disabled: e.count === 0, on: { click: () => void emptyFolder(s) } }, icon('trash'));
  }

  /** The summary for the person being shown, fetched once per set of addresses. */
  person: { key: string; summary: PersonSummary | null } | null = null;
  eventsOpen = false;

  async reload() {
    ++this.generation;
    this.cursor = -1;
    this.selected.clear();
    this.scroller.scrollTop = 0;
    this.renderHeader();
    await this.list.reset();
    const sel = store.state.selectedThread;
    // A new folder or search: the old conversation mustn't stay on screen,
    // or its buttons would act on whatever the cursor lands on.
    if (!sel && this.readerThread) this.renderEmptyReader();
    if (sel) {
      this.cursor = this.list.indexOf(sel);
      this.renderRows();
    }
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
      if (still) this.openSummary = still;
      if (still && still.message_count !== this.openThread.messages.length) void this.showThread(still.thread_id, false, true);
    }
  }

  toggleSelect(t: ThreadSummary) {
    if (this.selected.has(t.thread_id)) this.selected.delete(t.thread_id);
    else this.selected.set(t.thread_id, t);
    this.renderHeader();
    this.renderRows();
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
    // For the palette: whether there's a conversation to act on, or one open.
    const some = () => this.targetThreads().length > 0;
    const reading = () => !!this.openThread;
    bind('j', 'Next conversation', () => void this.move(1), s);
    bind('ArrowDown', 'Next conversation', () => void this.move(1), s);
    bind('k', 'Previous conversation', () => void this.move(-1), s);
    bind('ArrowUp', 'Previous conversation', () => void this.move(-1), s);
    bind('Enter', 'Open conversation', () => void this.openAtCursor(), s);
    bind('o', 'Open conversation', () => void this.openAtCursor(), s);
    bind('x', 'Select conversation', () => { const t = this.list.at(this.cursor); if (t) this.toggleSelect(t); }, s);
    bind('e', 'Archive', () => void this.act('archive'), s, some);
    bind('#', 'Delete', () => void this.act('trash'), s, some);
    bind('Backspace', 'Delete', () => void this.act('trash'), s);
    bind('Delete', 'Delete', () => void this.act('trash'), s);
    bind('u', 'Mark unread', () => void this.act('unread'), s, some);
    bind('shift+i', 'Mark read', () => void this.act('read'), s, some);
    bind('!', 'Mark as spam / Not spam', () => void this.act(this.inSpam() ? 'not_spam' : 'spam'), s, some);
    bind('s', 'Flag / unflag', () => { const t = this.targetThreads()[0]; if (t) void this.flagThread(t); }, s, some);
    bind('r', 'Reply', () => this.replyBox?.focus('reply'), s, reading);
    bind('a', 'Reply all', () => this.replyBox?.focus('reply_all'), s, reading);
    bind('f', 'Forward', () => void this.reply('forward'), s, reading);
    bind('v', 'Move to…', () => { const b = this.reader.querySelector<HTMLElement>('[aria-label="Move"]'); if (b) this.moveMenu(b); }, s, reading);
    bind('/', 'Search', () => { this.searchInput.focus(); this.searchInput.select(); }, s);
    bind('Escape', 'Close conversation', () => { store.set({ selectedThread: null }); this.renderEmptyReader(); this.renderRows(); this.scroller.focus(); }, s);
    commandSource(() => actions.paletteCommands(this), s);
  }

  // Helpers in the sibling modules, which take this view as their first argument.
  loadPerson(emails: string[]) { person.loadPerson(this, emails); }
  personCard(s: Extract<MailSource, { kind: 'person' }>) { return person.personCard(this, s); }
  scale() { return list.scale(this); }
  logicalTop() { return list.logicalTop(this); }
  visibleRange(pad?: number) { return list.visibleRange(this, pad); }
  scrollToIndex(i: number, offset?: number) { list.scrollToIndex(this, i, offset); }
  renderRows() { list.renderRows(this); }
  rowMenu(t: ThreadSummary, at: { x: number; y: number }): (MenuItem | '-')[] { return actions.rowMenu(this, t, at); }
  targetThreads() { return actions.targetThreads(this); }
  messageIds(threads: ThreadSummary[]) { return actions.messageIds(this, threads); }
  inSpam(threads?: ThreadSummary[]) { return actions.inSpam(this, threads); }
  act(kind: Action, threads?: ThreadSummary[]) { return actions.act(this, kind, threads); }
  flagThread(t: ThreadSummary) { return actions.flagThread(this, t); }
  unsubscribe(m: MessageDetail, inSpam: boolean) { return actions.unsubscribe(this, m, inSpam); }
  moveMenu(anchor: HTMLElement | { x: number; y: number }, threads?: ThreadSummary[]) { actions.moveMenu(this, anchor, threads); }
  moreMenu(anchor: HTMLElement, thread: Thread) { actions.moreMenu(this, anchor, thread); }
  renderEmptyReader() { reader.renderEmptyReader(this); }
  showThread(threadId: string, markRead?: boolean, reload?: boolean) { return reader.showThread(this, threadId, markRead, reload); }
  reply(mode: 'reply' | 'reply_all' | 'forward', messageId?: string) { return reader.reply(this, mode, messageId); }
}

export function addressList(list: { email: string; name?: string | null }[]) {
  return list.map(formatAddress).join(', ');
}

/** The settings that decide which remote images load, as one comparable value. */
function imageSettings() {
  const s = store.state.settings;
  return s ? `${s.load_remote_images}|${s.remote_image_senders.join(',')}` : '';
}
