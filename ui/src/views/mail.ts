// Mail: thread list (virtualized) + reading pane.

import { api } from '../api';
import { debounce, h, icon, replace } from '../lib/dom';
import { avatarColor, displayName, formatAddress, fullDate, humanSize, initials, listDate, participantsLabel, timeRange } from '../lib/format';
import { bind, confirmDialog, errorToast, modal, popupMenu, toast } from '../lib/ui';
import { store } from '../state';
import { transport } from '../transport';
import type { Mailbox, MessageDetail, PartStat, Thread, ThreadQuery, ThreadSummary } from '../types';
import { openComposer } from './composer';

const ROW_H = 74;
const PAGE = 100;

export class MailView {
  root = h('div.mail-view');
  private listPane = h('section.list-pane', { 'aria-label': 'Conversations' });
  private reader = h('section.reader', { 'aria-label': 'Message' });
  private scroller = h('div.list-scroll', { tabindex: 0, role: 'listbox', 'aria-label': 'Conversations' });
  private spacer = h('div.list-spacer');
  private rows = h('div.list-rows');
  private header = h('header.list-head');
  private searchInput = h('input.search', { type: 'search', placeholder: 'Search mail', 'aria-label': 'Search mail' });
  private threads: ThreadSummary[] = [];
  private exhausted = false;
  private loading = false;
  private cursor = -1;
  private selected = new Set<string>();
  private openThread: Thread | null = null;
  private generation = 0;

  constructor() {
    this.spacer.appendChild(this.rows);
    this.scroller.appendChild(this.spacer);
    this.listPane.append(this.header, this.scroller);
    this.root.append(this.listPane, this.reader);
    this.scroller.addEventListener('scroll', () => {
      this.renderRows();
      if (this.scroller.scrollTop + this.scroller.clientHeight > this.threads.length * ROW_H - ROW_H * 10) void this.loadMore();
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

  private query(before: string | null): ThreadQuery {
    const s = store.state.source;
    const q: ThreadQuery = { limit: PAGE, before };
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
    const gen = ++this.generation;
    this.threads = [];
    this.exhausted = false;
    this.cursor = -1;
    this.selected.clear();
    this.scroller.scrollTop = 0;
    this.renderHeader();
    this.renderRows();
    await this.loadMore(gen);
    if (store.state.selectedThread && this.threads.some((t) => t.thread_id === store.state.selectedThread)) {
      this.cursor = this.threads.findIndex((t) => t.thread_id === store.state.selectedThread);
      this.renderRows();
    }
  }

  /** Re-fetch the loaded range in place (after sync or local changes). */
  async refresh() {
    const gen = this.generation;
    try {
      const want = Math.max(PAGE, this.threads.length);
      const fresh = await api.listThreads({ ...this.query(null), limit: Math.min(want, 500) });
      if (gen !== this.generation) return;
      const cur = this.threads[this.cursor]?.thread_id;
      this.threads = fresh;
      this.exhausted = fresh.length < want;
      if (cur) this.cursor = this.threads.findIndex((t) => t.thread_id === cur);
      this.renderRows();
      if (this.openThread) {
        const still = this.threads.find((t) => t.thread_id === this.openThread!.thread_id);
        if (still && still.message_count !== this.openThread.messages.length) void this.showThread(still.thread_id, false);
      }
    } catch (e) {
      console.warn(e);
    }
  }

  private async loadMore(gen = this.generation) {
    if (this.loading || this.exhausted) return;
    this.loading = true;
    try {
      const last = this.threads[this.threads.length - 1];
      const page = await api.listThreads(this.query(last?.last_date ?? null));
      if (gen !== this.generation) return;
      const seen = new Set(this.threads.map((t) => t.thread_id));
      this.threads.push(...page.filter((t) => !seen.has(t.thread_id)));
      if (page.length < PAGE) this.exhausted = true;
      this.renderRows();
    } catch (e) {
      errorToast(e);
    } finally {
      this.loading = false;
    }
  }

  private renderRows() {
    this.spacer.style.height = `${Math.max(this.threads.length * ROW_H, 1)}px`;
    if (!this.threads.length) {
      replace(this.rows, h('div.empty-list', this.loading ? 'Loading…' : emptyMessage()));
      this.rows.style.transform = '';
      return;
    }
    const top = this.scroller.scrollTop;
    const first = Math.max(0, Math.floor(top / ROW_H) - 5);
    const last = Math.min(this.threads.length, Math.ceil((top + this.scroller.clientHeight) / ROW_H) + 5);
    this.rows.style.transform = `translateY(${first * ROW_H}px)`;
    const me = store.state.myEmails;
    replace(
      this.rows,
      this.threads.slice(first, last).map((t, i) => {
        const idx = first + i;
        const active = t.thread_id === store.state.selectedThread;
        const row = h(
          `div.thread-row${t.unread_count ? '.unread' : ''}${active ? '.active' : ''}${idx === this.cursor ? '.cursor' : ''}${this.selected.has(t.thread_id) ? '.selected' : ''}`,
          {
            role: 'option',
            'aria-selected': active ? 'true' : 'false',
            style: { height: `${ROW_H}px` },
            on: {
              click: (e: MouseEvent) => {
                if (e.metaKey || e.ctrlKey || e.shiftKey) {
                  this.toggleSelect(t.thread_id);
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
        );
        return row;
      }),
    );
  }

  private toggleSelect(id: string) {
    if (this.selected.has(id)) this.selected.delete(id);
    else this.selected.add(id);
    this.renderHeader();
    this.renderRows();
  }

  private targetThreads(): ThreadSummary[] {
    if (this.selected.size) return this.threads.filter((t) => this.selected.has(t.thread_id));
    const t = this.threads.find((x) => x.thread_id === store.state.selectedThread) ?? this.threads[this.cursor];
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
        const nextIdx = Math.min(this.cursor, this.threads.length - 1 - removed.size);
        this.threads = this.threads.filter((t) => !removed.has(t.thread_id));
        this.selected.clear();
        this.cursor = Math.max(0, nextIdx);
        const next = this.threads[this.cursor];
        this.renderHeader();
        this.renderRows();
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
        const t = this.threads.find((x) => x.thread_id === threadId);
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
    const last = thread.messages.length - 1;
    replace(
      this.reader,
      h('header.reader-head',
        h('div.reader-actions',
          h('button.icon-btn', { title: 'Archive (e)', 'aria-label': 'Archive', on: { click: () => void this.act('archive') } }, icon('archive')),
          h('button.icon-btn', { title: 'Delete (#)', 'aria-label': 'Delete', on: { click: () => void this.act('trash') } }, icon('trash')),
          h('button.icon-btn', { title: 'Mark unread (u)', 'aria-label': 'Mark unread', on: { click: () => void this.act('unread') } }, icon('mail')),
          h('button.icon-btn', { title: 'Move to… (v)', 'aria-label': 'Move', on: { click: (e: MouseEvent) => this.moveMenu(e.currentTarget as HTMLElement) } }, icon('folder')),
          h('button.icon-btn', { title: 'More', 'aria-label': 'More actions', on: { click: (e: MouseEvent) => this.moreMenu(e.currentTarget as HTMLElement, thread) } }, icon('more'))),
        h('h2.reader-subject', thread.subject || '(no subject)'),
        labels.length > 0 && h('div.labels', labels.map((m) => h('span.label-chip', m.name)))),
      h('div.messages', thread.messages.map((m, i) => this.renderMessage(m, i === last || !m.flags.seen))),
      h('div.reply-bar',
        h('button', { on: { click: () => void this.reply('reply') } }, icon('reply', 16), 'Reply'),
        h('button', { on: { click: () => void this.reply('reply_all') } }, icon('replyAll', 16), 'Reply all'),
        h('button', { on: { click: () => void this.reply('forward') } }, icon('forward', 16), 'Forward')),
    );
    this.reader.scrollTop = 0;
    const lastEl = this.reader.querySelector('.message.expanded:last-of-type');
    if (thread.messages.length > 2) lastEl?.scrollIntoView({ block: 'start' });
  }

  private renderMessage(m: MessageDetail, expanded: boolean): HTMLElement {
    const card = h(`article.message${expanded ? '.expanded' : ''}`);
    const head = h('header.message-head', {
      on: { click: (e: MouseEvent) => { if ((e.target as HTMLElement).closest('button')) return; card.classList.toggle('expanded'); if (card.classList.contains('expanded') && !card.querySelector('.message-body')) fill(); } },
    },
      h('span.avatar', { style: { background: avatarColor(m.from?.email ?? '') } }, initials(m.from)),
      h('div.message-meta',
        h('div.from', h('strong', displayName(m.from)), m.from && h('span.muted', ` <${m.from.email}>`)),
        h('div.to.muted.small', `to ${[...m.to, ...m.cc].map((a) => (store.state.myEmails.has(a.email.toLowerCase()) ? 'me' : displayName(a))).join(', ') || '(no recipients)'}`),
        h('div.collapsed-snippet.muted', m.snippet)),
      h('time.muted.small', { datetime: m.date, title: fullDate(m.date) }, listDate(m.date)),
      h('button.icon-btn', { title: 'Reply', 'aria-label': 'Reply to this message', on: { click: () => void this.reply('reply', m.id) } }, icon('reply', 16)),
    );
    card.appendChild(head);
    const fill = () => {
      const bodyWrap = h('div.message-body');
      card.appendChild(bodyWrap);
      if (m.body_pending) bodyWrap.appendChild(h('p.muted', 'Downloading message…'));
      if (m.remote_images_blocked) {
        bodyWrap.appendChild(
          h('div.banner',
            icon('image', 16),
            h('span', 'Remote images are hidden to protect your privacy.'),
            h('button.link', { on: { click: () => void this.loadImages(m, false, card) } }, 'Show once'),
            h('button.link', { on: { click: () => void this.loadImages(m, true, card) } }, `Always for ${m.from?.email ?? 'sender'}`)),
        );
      }
      if (m.invite) bodyWrap.appendChild(inviteCard(m));
      bodyWrap.appendChild(m.html_body ? htmlFrame(m.html_body) : textBody(m.text_body));
      const atts = m.attachments.filter((a) => !a.inline);
      if (atts.length) {
        bodyWrap.appendChild(
          h('div.attachments',
            atts.map((a) =>
              h('button.attachment', { title: `Download ${a.filename}`, on: { click: () => void download(m.id, a.id) } },
                icon('paperclip', 16), h('span.name', a.filename), h('span.muted.small', humanSize(a.size))),
            )),
        );
      }
    };
    if (expanded) fill();
    return card;
  }

  private async loadImages(m: MessageDetail, always: boolean, card: HTMLElement) {
    try {
      const fresh = await api.loadRemoteImages(m.id, always);
      card.replaceWith(this.renderMessage(fresh, true));
    } catch (e) {
      errorToast(e);
    }
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
      '-',
      { label: 'View source', icon: 'draft', run: () => void viewSource(lastMsg.id) },
    ]);
  }

  private move(delta: number) {
    if (!this.threads.length) return;
    this.cursor = Math.max(0, Math.min(this.threads.length - 1, this.cursor + delta));
    const y = this.cursor * ROW_H;
    if (y < this.scroller.scrollTop) this.scroller.scrollTop = y;
    else if (y + ROW_H > this.scroller.scrollTop + this.scroller.clientHeight) this.scroller.scrollTop = y + ROW_H - this.scroller.clientHeight;
    this.renderRows();
    if (store.state.selectedThread) void this.showThread(this.threads[this.cursor].thread_id);
  }

  private bindKeys() {
    const s = 'mail';
    bind('j', 'Next conversation', () => this.move(1), s);
    bind('ArrowDown', 'Next conversation', () => this.move(1), s);
    bind('k', 'Previous conversation', () => this.move(-1), s);
    bind('ArrowUp', 'Previous conversation', () => this.move(-1), s);
    bind('Enter', 'Open conversation', () => { const t = this.threads[this.cursor]; if (t) void this.showThread(t.thread_id); }, s);
    bind('o', 'Open conversation', () => { const t = this.threads[this.cursor]; if (t) void this.showThread(t.thread_id); }, s);
    bind('x', 'Select conversation', () => { const t = this.threads[this.cursor]; if (t) this.toggleSelect(t.thread_id); }, s);
    bind('e', 'Archive', () => void this.act('archive'), s);
    bind('#', 'Delete', () => void this.act('trash'), s);
    bind('Backspace', 'Delete', () => void this.act('trash'), s);
    bind('u', 'Mark unread', () => void this.act('unread'), s);
    bind('shift+i', 'Mark read', () => void this.act('read'), s);
    bind('!', 'Mark as spam', () => void this.act('spam'), s);
    bind('s', 'Flag / unflag', () => { const t = this.targetThreads()[0]; if (t) void this.flagThread(t); }, s);
    bind('r', 'Reply', () => void this.reply('reply'), s);
    bind('a', 'Reply all', () => void this.reply('reply_all'), s);
    bind('f', 'Forward', () => void this.reply('forward'), s);
    bind('v', 'Move to…', () => { const b = this.reader.querySelector<HTMLElement>('[aria-label="Move"]'); if (b) this.moveMenu(b); }, s);
    bind('/', 'Search', () => { this.searchInput.focus(); this.searchInput.select(); }, s);
    bind('Escape', 'Close conversation', () => { store.set({ selectedThread: null }); this.renderEmptyReader(); this.renderRows(); this.scroller.focus(); }, s);
  }
}

function inviteCard(m: MessageDetail): HTMLElement {
  const inv = m.invite!;
  const when = `${new Date(inv.start).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })} · ${timeRange(inv.start, inv.end, inv.all_day)}`;
  if (inv.method === 'CANCEL') {
    return h('div.invite.cancelled', icon('calendar', 20), h('div', h('strong', `Cancelled: ${inv.title}`), h('div.muted.small', when)));
  }
  if (inv.method !== 'REQUEST') {
    return h('div.invite', icon('calendar', 20), h('div', h('strong', inv.title), h('div.muted.small', when)));
  }
  const card = h('div.invite');
  const render = (status: PartStat | null) => {
    replace(card,
      icon('calendar', 22),
      h('div.grow',
        h('strong', inv.title),
        h('div.muted.small', when),
        inv.location && h('div.muted.small', icon('location', 12), ` ${inv.location}`),
        inv.organizer && h('div.muted.small', `From ${inv.organizer.name ?? inv.organizer.email}`)),
      h('div.rsvp-buttons',
        (['accepted', 'tentative', 'declined'] as PartStat[]).map((r) =>
          h(`button${status === r ? '.active' : ''}`, {
            on: {
              click: async () => {
                try {
                  const res = await api.respondToEmailInvite(m.id, r);
                  render(r);
                  toast(res.via === 'calendar' ? 'Response saved to your calendar' : 'Reply sent to the organizer');
                } catch (e) {
                  errorToast(e);
                }
              },
            },
          }, r === 'accepted' ? 'Yes' : r === 'declined' ? 'No' : 'Maybe'))));
  };
  render(inv.my_status);
  return card;
}

function emptyMessage(): string {
  const s = store.state.source;
  if (!store.state.accounts.length) return 'No accounts yet — add one in Settings.';
  if (s.kind === 'search') return 'No matching conversations.';
  return s.kind === 'role' && s.role === 'inbox' ? 'Inbox zero. 🎉' : 'Nothing here.';
}

function linkified(line: string): HTMLElement {
  const div = h(line.startsWith('>') ? 'div.quote' : 'div');
  for (const p of line.split(/(https?:\/\/[^\s<>"]+)/g)) {
    if (/^https?:\/\//.test(p)) div.appendChild(h('a', { href: p, rel: 'noopener noreferrer', target: '_blank', on: { click: (e: MouseEvent) => { e.preventDefault(); openExternal(p); } } }, p));
    else if (p) div.appendChild(document.createTextNode(p));
  }
  if (!line) div.appendChild(h('br'));
  return div;
}

const isAttribution = (l: string) => /^On .+wrote:\s*$/.test(l.trim()) || /^-{2,} ?(Original|Forwarded) [Mm]essage/.test(l.trim());

/** Plain text body: linkified, with quoted history folded behind a toggle. */
function textBody(text: string): HTMLElement {
  const pre = h('div.text-body');
  const lines = text.replace(/\s+$/, '').split('\n');
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (isAttribution(line) || line.startsWith('>')) {
      const start = i;
      while (i < lines.length && (lines[i].startsWith('>') || isAttribution(lines[i]) || (lines[i].trim() === '' && lines[i + 1]?.startsWith('>')))) i++;
      // Only fold real history, not a single quoted line.
      if (i - start >= 2 || isAttribution(lines[start])) {
        const block = h('div.quoted', { hidden: true }, lines.slice(start, i).map(linkified));
        pre.append(h('button.quote-toggle', { title: 'Show quoted text', 'aria-label': 'Show quoted text', on: { click: (e: MouseEvent) => { block.hidden = !block.hidden; (e.currentTarget as HTMLElement).classList.toggle('open'); } } }, '•••'), block);
        continue;
      }
      i = start;
    }
    pre.appendChild(linkified(line));
    i++;
  }
  return pre;
}

/** Sanitized HTML (from the core) in a script-less sandboxed iframe with a CSP. */
function htmlFrame(html: string): HTMLElement {
  const csp = "default-src 'none'; img-src data: https: http:; style-src 'unsafe-inline'; font-src data:";
  const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><base target="_blank"><style>
    html,body{margin:0;padding:0;background:transparent;color:#1d1d1f;font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;overflow-wrap:anywhere}
    img{max-width:100%;height:auto} table{max-width:100%} blockquote{border-left:3px solid #ccc;margin:0 0 0 .5em;padding-left:.75em;color:#555}
    a{color:#2a62d9}
  </style></head><body>${html}</body></html>`;
  const frame = h('iframe.html-body', { sandbox: 'allow-same-origin allow-popups allow-popups-to-escape-sandbox', referrerpolicy: 'no-referrer', title: 'Message content' }) as HTMLIFrameElement;
  frame.srcdoc = doc;
  frame.addEventListener('load', () => {
    const d = frame.contentDocument;
    if (!d) return;
    const fit = () => (frame.style.height = `${Math.min(d.documentElement.scrollHeight + 4, 20000)}px`);
    fit();
    new ResizeObserver(fit).observe(d.body);
    d.addEventListener('click', (e) => {
      const a = (e.target as HTMLElement).closest('a');
      if (a?.href && /^(https?|mailto):/.test(a.href)) {
        e.preventDefault();
        if (a.href.startsWith('mailto:')) {
          const email = decodeURIComponent(a.href.slice(7).split('?')[0]);
          openComposer({ to: [{ email }] });
        } else openExternal(a.href);
      }
    });
  });
  return h('div.html-wrap', frame);
}

export function openExternal(url: string) {
  if (transport.kind === 'tauri') void transport.invoke('open_url', { url }).catch(() => window.open(url, '_blank', 'noopener'));
  else window.open(url, '_blank', 'noopener,noreferrer');
}

async function download(messageId: string, attachmentId: string) {
  try {
    const a = await api.getAttachment(messageId, attachmentId);
    await saveBase64(a.filename, a.mime_type, a.data_base64);
  } catch (e) {
    errorToast(e);
  }
}

export async function saveBase64(filename: string, mime: string, b64: string) {
  if (transport.kind === 'tauri') {
    const path = await transport.invoke<string | null>('save_file', { filename, data_base64: b64 });
    if (path) toast(`Saved to ${path}`, { kind: 'success' });
    return;
  }
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const url = URL.createObjectURL(new Blob([bytes], { type: mime }));
  const link = h('a', { href: url, download: filename });
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

async function viewSource(messageId: string) {
  try {
    const { raw } = await api.getRaw(messageId);
    modal('Message source', h('pre.source', raw), { wide: true });
  } catch (e) {
    errorToast(e);
  }
}

export function addressList(list: { email: string; name?: string | null }[]) {
  return list.map(formatAddress).join(', ');
}
