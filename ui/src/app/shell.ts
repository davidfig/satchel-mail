import { api } from '../api';
import { transport } from '../transport';
import { h, icon, patch, replace } from '../lib/dom';
import { deleteFolder, emptiable, emptyFolder, folderDialog, folderTree } from '../lib/folders';
import { fullDate, relative, relativeShort, shortProgress } from '../lib/format';
import { bind, contextMenu, errorToast, setScope, toast } from '../lib/ui';
import type { MenuItem } from '../lib/ui';
import { type MailSource, type View, store } from '../state';
import type { Account, Mailbox, MailboxRole } from '../types';
import { CalendarView } from '../views/calendar';
import { ContactsView } from '../views/contacts';
import { openComposer } from '../views/mail/composer';
import { MailView } from '../views/mail';
import { ReviewView } from '../views/review';
import { SettingsView } from '../views/settings';
import { showShortcuts } from './help';
import { COLLAPSED_KEY, LOCATION_KEY, SECTIONS_KEY, UNIFIED, readCollapsed, readJson, writeJson } from './location';
import { palette } from './palette';

/** Each account's built-in folders, in sidebar order: the ones the
 *  combined list under Mail shows, then Gmail's Important and All Mail. */
const BUILT_IN: { role: MailboxRole; icon: string }[] = [
  ...UNIFIED,
  { role: 'important', icon: 'label' },
  { role: 'all', icon: 'archive' },
];

/** The app frame: sidebar navigation and the current view. */
export class Shell {
  private sidebar = h('nav.sidebar', { 'aria-label': 'Navigation' });
  private content = h('main.content');
  private app = h('div.app', this.sidebar, this.content);
  private views: Partial<Record<View, { root: HTMLElement; show?: () => void }>> = {};
  /** Sidebar account sections the user has shrunk, by account id. */
  private closedSections = new Set(readJson<string[]>(SECTIONS_KEY) ?? []);

  constructor() {
    this.app.classList.toggle('collapsed', readCollapsed());
    document.body.append(this.app);
    store.subscribe((_, changed) => {
      if (changed.has('view')) this.showView();
      if (changed.has('view') || changed.has('source')) writeJson(LOCATION_KEY, { view: store.state.view, source: store.state.source });
      if (['view', 'accounts', 'mailboxes', 'pendingProposals', 'source'].some((k) => changed.has(k as never))) this.renderSidebar();
    });
    this.renderSidebar();
    this.showView();
    this.bindKeys();
  }

  toggleSidebar() {
    const collapsed = this.app.classList.toggle('collapsed');
    try {
      localStorage.setItem(COLLAPSED_KEY, collapsed ? '1' : '0');
    } catch {
      /* remembering is a convenience */
    }
    this.renderSidebar();
  }

  private toggleSection(id: string) {
    if (!this.closedSections.delete(id)) this.closedSections.add(id);
    writeJson(SECTIONS_KEY, [...this.closedSections]);
    this.renderSidebar();
  }

  /** A section header that expands or shrinks the items under it. */
  private sectionHeader(id: string, ...label: (Node | string | null)[]) {
    const open = !this.closedSections.has(id);
    const el = h('button.mb-section.mb-toggle', { 'aria-expanded': String(open), on: { click: () => this.toggleSection(id) } },
      icon(open ? 'chevronDown' : 'chevronRight', 12), ...label);
    const account = store.state.accounts.find((a) => a.id === id);
    contextMenu(el, () => [
      { label: open ? 'Collapse' : 'Expand', icon: open ? 'chevronRight' : 'chevronDown', run: () => this.toggleSection(id) },
      ...(account ? [{ label: 'New folder…', icon: 'plus', run: () => void this.newFolder(account.id, null) }] : []),
      '-',
      { label: account ? 'Sync this account now' : 'Sync all accounts now', icon: 'refresh', run: () => this.syncNow(account?.id ?? null) },
      ...(account
        ? [
            { label: `Compose from ${account.email}`, icon: 'compose', run: () => openComposer({ account_id: account.id }) },
            { label: 'Account settings', icon: 'settings', run: () => this.go('settings') },
          ]
        : []),
    ]);
    return el;
  }

  private syncNow(accountId: string | null) {
    api.syncNow(accountId).then(() => toast('Syncing…', { quiet: true }), errorToast);
  }

  /** Right-click menu for a mailbox in the sidebar. */
  private sourceMenu(source: MailSource, unread: number): (MenuItem | '-')[] {
    const mb = source.kind === 'mailbox' ? store.mailbox(source.mailbox_id) : undefined;
    const role = source.kind === 'role' ? source.role : null;
    const markRead = async () => {
      try {
        const { message_ids } = await api.markAllRead(mb?.id ?? null, role);
        if (!message_ids.length) return;
        toast(`Marked ${message_ids.length} message${message_ids.length === 1 ? '' : 's'} as read`, {
          action: { label: 'Undo', run: () => void api.setFlags(message_ids, false, null).catch(errorToast) },
        });
      } catch (e) {
        errorToast(e);
      }
    };
    // Where "New folder" goes: this account, or the only one.
    const accountId = mb?.account_id ?? (store.state.accounts.length === 1 ? store.state.accounts[0].id : null);
    const kind = mb?.is_label ? 'label' : 'folder';
    const folderItems: (MenuItem | '-')[] = [
      ...(accountId ? ['-' as const, { label: `New ${kind}…`, icon: 'plus', run: () => void this.newFolder(accountId, null) }] : []),
      ...(mb?.role === 'custom'
        ? [
            { label: `New ${kind} inside…`, icon: 'folder', run: () => void this.newFolder(mb.account_id, mb.id) },
            { label: 'Rename or move…', icon: 'draft', run: () => void this.editFolder(mb) },
            { label: `Delete ${kind}…`, icon: 'trash', danger: true, run: () => void this.removeFolder(mb) },
          ]
        : []),
    ];
    const empty = emptiable(source);
    return [
      { label: 'Open', icon: 'mailOpen', run: () => this.go('mail', source) },
      // Drafts show a total, not unread.
      ...(role !== 'drafts' ? [{ label: 'Mark all as read', icon: 'check', disabled: unread === 0, run: () => void markRead() }] : []),
      ...(empty ? [{ label: `Empty ${empty.label}…`, icon: 'trash', danger: true, disabled: empty.count === 0, run: () => void emptyFolder(source) }] : []),
      '-',
      { label: mb ? 'Sync this account now' : 'Sync all accounts now', icon: 'refresh', run: () => this.syncNow(mb?.account_id ?? null) },
      { label: 'Compose', icon: 'compose', hint: 'c', run: () => openComposer(mb ? { account_id: mb.account_id } : {}) },
      ...folderItems,
    ];
  }

  private view(v: View) {
    if (!this.views[v]) {
      this.views[v] = v === 'mail' ? new MailView() : v === 'calendar' ? new CalendarView() : v === 'contacts' ? new ContactsView() : v === 'review' ? new ReviewView() : new SettingsView();
    }
    return this.views[v]!;
  }

  private showView() {
    const v = this.view(store.state.view);
    setScope(store.state.view);
    replace(this.content, v.root);
    v.show?.();
  }

  go(view: View, source?: MailSource) {
    store.set(source ? { view, source, selectedThread: null } : { view });
  }

  private navItem(view: View, label: string, ic: string, badge?: number, parent = false) {
    const active = store.state.view === view;
    return h(`button.nav-item${active ? '.active' : ''}${parent ? '.parent' : ''}`, { title: label, 'aria-label': label, 'aria-current': active ? 'page' : undefined, on: { click: () => this.go(view) } },
      icon(ic), h('span', label), badge ? h('span.badge', String(badge)) : null);
  }

  private sourceItem(source: MailSource, ic: string, count: number, opts: { text?: string; depth?: number } = {}) {
    const s = store.state.source;
    // With one account, "Inbox" (g i, the default) is that account's Inbox.
    const sole = store.state.accounts.length === 1 && s.kind === 'role' && source.kind === 'mailbox' && store.mailbox(source.mailbox_id)?.role === s.role;
    const active = store.state.view === 'mail' && (sole || (s.kind === source.kind && (s.kind === 'role' ? s.role === (source as { role: string }).role : s.kind === 'mailbox' && s.mailbox_id === (source as { mailbox_id: string }).mailbox_id)));
    const el = h(`button.mb-item${active ? '.active' : ''}`, { title: source.label, style: opts.depth ? { paddingLeft: `${10 + opts.depth * 16}px` } : undefined, on: { click: () => this.go('mail', source) } },
      icon(ic, 16),
      h('span.mb-name', opts.text ?? source.label),
      count > 0 && h('span.mb-count', String(count)));
    contextMenu(el, () => this.sourceMenu(source, count));
    return el;
  }

  private renderSidebar() {
    const { accounts, mailboxes, pendingProposals } = store.state;
    const unread = (role: MailboxRole) => mailboxes.filter((m) => m.role === role).reduce((n, m) => n + (role === 'drafts' ? m.total : m.unread), 0);
    const syncing = accounts.filter((a) => a.sync.state === 'syncing');
    const collapsed = this.app.classList.contains('collapsed');
    const errors = accounts.filter((a) => a.sync.state === 'error' || a.sync.state === 'offline');
    const combined = store.state.view === 'mail' && !collapsed && accounts.length > 1;
    const inboxUnread = unread('inbox');
    transport.setTitle(inboxUnread > 0 ? `(${inboxUnread}) satchel.email` : 'satchel.email');
    // Patched, not replaced: this runs on every sync tick, and swapping the
    // nodes out from under the pointer made hover and tooltips flicker.
    patch(
      this.sidebar,
      h('div.brand', h('img.logo', { src: '/favicon.svg', alt: '' }), h('span.brand-name', 'satchel.email'),
        h('button.icon-btn.collapse-btn', {
          title: `${collapsed ? 'Expand' : 'Collapse'} sidebar (${navigator.platform.includes('Mac') ? '⌘' : 'Ctrl+'}\\)`,
          'aria-label': collapsed ? 'Expand sidebar' : 'Collapse sidebar',
          'aria-expanded': String(!collapsed),
          on: { click: () => this.toggleSidebar() },
        }, icon(collapsed ? 'chevronRight' : 'chevronLeft', 16))),
      h('button.primary.compose-btn', { title: 'Compose (c)', 'aria-label': 'Compose', on: { click: () => openComposer({}) } }, icon('compose', 16), h('span', 'Compose')),
      h('div.nav',
        this.navItem('mail', 'Mail', 'mail', undefined, combined),
        // Every account's Inbox, Flagged, … together, under Mail. With one
        // account its own section below says it all.
        combined && h('div.nav-sub',
          UNIFIED.filter((u) => mailboxes.some((m) => m.role === u.role) || u.role === 'inbox').map((u) =>
            this.sourceItem({ kind: 'role', role: u.role, label: u.label }, u.icon, unread(u.role), { depth: 1 }))),
        this.navItem('calendar', 'Calendar', 'calendar'),
        this.navItem('contacts', 'Contacts', 'user'),
        this.navItem('review', 'AI Proposals', 'sparkle', pendingProposals),
        this.navItem('settings', 'Settings', 'settings')),
      store.state.view === 'mail' && !collapsed &&
        h('div.mailboxes',
          accounts.map((a) => this.accountSection(a, mailboxes.filter((m) => m.account_id === a.id)))),
      h('div.sidebar-foot',
        errors.length
          ? h('button.sync-status.error', { title: errors.map((a) => `${a.email}: ${a.sync.error}`).join('\n'), on: { click: () => this.go('settings') } }, icon('warning', 14), `${errors.length} account${errors.length > 1 ? 's' : ''} need attention`)
          : syncing.length
            // One row per syncing account, marked with its color.
            ? syncing.map((a) => h('div.sync-status', { title: `${a.email}: ${a.sync.detail ?? 'Syncing…'}` },
                h('span.acct-dot.busy', { style: { background: a.color } }), h('span.sync-text', a.sync.detail ?? 'Syncing…')))
            : h('div.sync-status', icon('check', 14),
                accounts.length ? `Updated ${relative(accounts.map((a) => a.sync.last_success).filter(Boolean).sort().pop() ?? null)}` : 'No accounts')),
    );
    // Progress too wide for the sidebar drops its noun ("Downloading: 31,295
    // left") before it gets cut off; the tooltip keeps the full text.
    for (const t of this.sidebar.querySelectorAll<HTMLElement>('.sync-text')) {
      if (t.scrollWidth > t.clientWidth) t.textContent = shortProgress(t.textContent ?? '');
    }
  }

  private accountSection(a: Account, boxes: Mailbox[]) {
    const err = a.sync.state === 'error' || a.sync.state === 'offline';
    const open = !this.closedSections.has(a.id);
    const labels = boxes.some((m) => m.is_label);
    return h('div.account-section',
      h('div.mb-section-row',
        this.sectionHeader(a.id, h(`span.acct-dot${a.sync.state === 'syncing' ? '.busy' : ''}`, { style: { background: a.color } }), h('span.acct-email', a.email),
          h('span.acct-synced', { title: a.sync.last_success ? `Last synced ${fullDate(a.sync.last_success)}` : 'Not synced yet' }, relativeShort(a.sync.last_success)),
          err ? h('span.state-dot.err', { title: a.sync.state === 'offline' ? 'Offline' : 'Sync error' }) : null),
        h('button.icon-btn.mb-add', { title: labels ? 'New label' : 'New folder', on: { click: () => void this.newFolder(a.id, null) } }, icon('plus', 14))),
      open && BUILT_IN.flatMap(({ role, icon: ic }) => boxes.filter((m) => m.role === role).map((m) =>
        this.sourceItem({ kind: 'mailbox', mailbox_id: m.id, label: m.name }, ic, role === 'drafts' ? m.total : role === 'all' || role === 'sent' || role === 'trash' ? 0 : m.unread))),
      open && folderTree(boxes).map(({ box, depth }) =>
        this.sourceItem({ kind: 'mailbox', mailbox_id: box.id, label: box.path }, box.is_label ? 'label' : 'folder', box.unread, { text: box.name, depth })));
  }

  private async newFolder(accountId: string, parentId: string | null) {
    const f = await folderDialog(accountId, { parentId });
    if (f) this.go('mail', { kind: 'mailbox', mailbox_id: f.id, label: f.path });
  }

  private async editFolder(f: Mailbox) {
    const saved = await folderDialog(f.account_id, { folder: f });
    const s = store.state.source;
    if (saved && s.kind === 'mailbox' && s.mailbox_id === f.id) store.set({ source: { ...s, label: saved.path } });
  }

  private async removeFolder(f: Mailbox) {
    const s = store.state.source;
    if ((await deleteFolder(f)) && s.kind === 'mailbox' && s.mailbox_id === f.id) this.go('mail', { kind: 'role', role: 'inbox', label: 'Inbox' });
  }

  private bindKeys() {
    bind('c', 'Compose', () => openComposer({}));
    bind('g i', 'Go to Inbox', () => this.go('mail', { kind: 'role', role: 'inbox', label: 'Inbox' }));
    bind('g s', 'Go to Flagged', () => this.go('mail', { kind: 'role', role: 'flagged', label: 'Flagged' }));
    bind('g t', 'Go to Sent', () => this.go('mail', { kind: 'role', role: 'sent', label: 'Sent' }));
    bind('g d', 'Go to Drafts', () => this.go('mail', { kind: 'role', role: 'drafts', label: 'Drafts' }));
    bind('g c', 'Go to Calendar', () => this.go('calendar'));
    bind('g p', 'Go to Contacts (people)', () => this.go('contacts'));
    bind('g r', 'Go to AI Proposals', () => this.go('review'));
    bind('g ,', 'Go to Settings', () => this.go('settings'));
    bind('mod+k', 'Command palette', () => this.palette());
    bind('mod+\\', 'Collapse or expand sidebar', () => this.toggleSidebar());
    bind('?', 'Keyboard shortcuts', () => showShortcuts());
  }

  private palette() {
    palette(this);
  }
}
