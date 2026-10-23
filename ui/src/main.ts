import './styles.css';

import { api } from './api';
import { transport } from './transport';
import { h, icon, replace } from './lib/dom';
import { relative } from './lib/format';
import { bind, errorToast, installShortcuts, modal, setScope, shortcutList } from './lib/ui';
import { type MailSource, type View, applyTheme, store } from './state';
import type { Account, Mailbox, MailboxRole } from './types';
import { CalendarView } from './views/calendar';
import { openComposer } from './views/composer';
import { MailView } from './views/mail';
import { ReviewView } from './views/review';
import { SettingsView } from './views/settings';

const UNIFIED: { role: MailboxRole; label: string; icon: string }[] = [
  { role: 'inbox', label: 'Inbox', icon: 'inbox' },
  { role: 'flagged', label: 'Flagged', icon: 'star' },
  { role: 'drafts', label: 'Drafts', icon: 'draft' },
  { role: 'sent', label: 'Sent', icon: 'send' },
  { role: 'archive', label: 'Archive', icon: 'archive' },
  { role: 'junk', label: 'Spam', icon: 'junk' },
  { role: 'trash', label: 'Trash', icon: 'trash' },
];

class Shell {
  private sidebar = h('nav.sidebar', { 'aria-label': 'Navigation' });
  private content = h('main.content');
  private views: Partial<Record<View, { root: HTMLElement; show?: () => void }>> = {};

  constructor() {
    document.body.append(h('div.app', this.sidebar, this.content));
    store.subscribe((_, changed) => {
      if (changed.has('view')) this.showView();
      if (['view', 'accounts', 'mailboxes', 'pendingProposals', 'source'].some((k) => changed.has(k as never))) this.renderSidebar();
    });
    this.renderSidebar();
    this.showView();
    this.bindKeys();
  }

  private view(v: View) {
    if (!this.views[v]) {
      this.views[v] = v === 'mail' ? new MailView() : v === 'calendar' ? new CalendarView() : v === 'review' ? new ReviewView() : new SettingsView();
    }
    return this.views[v]!;
  }

  private showView() {
    const v = this.view(store.state.view);
    setScope(store.state.view);
    replace(this.content, v.root);
    v.show?.();
  }

  private go(view: View, source?: MailSource) {
    store.set(source ? { view, source, selectedThread: null } : { view });
  }

  private navItem(view: View, label: string, ic: string, badge?: number) {
    const active = store.state.view === view;
    return h(`button.nav-item${active ? '.active' : ''}`, { 'aria-current': active ? 'page' : undefined, on: { click: () => this.go(view) } },
      icon(ic), h('span', label), badge ? h('span.badge', String(badge)) : null);
  }

  private sourceItem(source: MailSource, ic: string, count: number, color?: string) {
    const s = store.state.source;
    const active = store.state.view === 'mail' && s.kind === source.kind && (s.kind === 'role' ? s.role === (source as { role: string }).role : s.kind === 'mailbox' && s.mailbox_id === (source as { mailbox_id: string }).mailbox_id);
    return h(`button.mb-item${active ? '.active' : ''}`, { on: { click: () => this.go('mail', source) } },
      color ? h('span.dot', { style: { background: color } }) : icon(ic, 16),
      h('span.mb-name', source.label),
      count > 0 && h('span.mb-count', String(count)));
  }

  private renderSidebar() {
    const { accounts, mailboxes, pendingProposals } = store.state;
    const unread = (role: MailboxRole) => mailboxes.filter((m) => m.role === role).reduce((n, m) => n + (role === 'drafts' ? m.total : m.unread), 0);
    const syncing = accounts.some((a) => a.sync.state === 'syncing');
    const errors = accounts.filter((a) => a.sync.state === 'error' || a.sync.state === 'offline');
    replace(
      this.sidebar,
      h('div.brand', h('span.logo', '◆'), h('span', 'Satchel')),
      h('button.primary.compose-btn', { on: { click: () => openComposer({}) } }, icon('compose', 16), 'Compose'),
      h('div.nav',
        this.navItem('mail', 'Mail', 'mail'),
        this.navItem('calendar', 'Calendar', 'calendar'),
        this.navItem('review', 'Review', 'sparkle', pendingProposals),
        this.navItem('settings', 'Settings', 'settings')),
      store.state.view === 'mail' &&
        h('div.mailboxes',
          h('div.mb-section', accounts.length > 1 ? 'All accounts' : ''),
          UNIFIED.filter((u) => mailboxes.some((m) => m.role === u.role) || u.role === 'inbox').map((u) =>
            this.sourceItem({ kind: 'role', role: u.role, label: u.label }, u.icon, unread(u.role))),
          accounts.map((a) => this.accountSection(a, mailboxes.filter((m) => m.account_id === a.id)))),
      h('div.sidebar-foot',
        errors.length
          ? h('button.sync-status.error', { title: errors.map((a) => `${a.email}: ${a.sync.error}`).join('\n'), on: { click: () => this.go('settings') } }, icon('warning', 14), `${errors.length} account${errors.length > 1 ? 's' : ''} need attention`)
          : h('div.sync-status', syncing ? h('span.spinner') : icon('check', 14),
              syncing ? (accounts.find((a) => a.sync.detail)?.sync.detail ?? 'Syncing…') : accounts.length ? `Updated ${relative(accounts.map((a) => a.sync.last_success).filter(Boolean).sort().pop() ?? null)}` : 'No accounts')),
    );
  }

  private accountSection(a: Account, boxes: Mailbox[]) {
    const custom = boxes.filter((m) => m.role === 'custom');
    if (!custom.length && store.state.accounts.length < 2) return null;
    const dot = a.sync.state === 'error' || a.sync.state === 'offline' ? '.err' : a.sync.state === 'syncing' ? '.busy' : '';
    return h('div.account-section',
      h('div.mb-section', h('span.acct-dot', { style: { background: a.color } }), h('span', a.email), h(`span.state-dot${dot}`)),
      store.state.accounts.length > 1 && boxes.filter((m) => m.role === 'inbox').map((m) => this.sourceItem({ kind: 'mailbox', mailbox_id: m.id, label: 'Inbox' }, 'inbox', m.unread)),
      custom.map((m) => this.sourceItem({ kind: 'mailbox', mailbox_id: m.id, label: m.path }, m.is_label ? 'label' : 'folder', m.unread)));
  }

  private bindKeys() {
    bind('c', 'Compose', () => openComposer({}));
    bind('g i', 'Go to Inbox', () => this.go('mail', { kind: 'role', role: 'inbox', label: 'Inbox' }));
    bind('g s', 'Go to Flagged', () => this.go('mail', { kind: 'role', role: 'flagged', label: 'Flagged' }));
    bind('g t', 'Go to Sent', () => this.go('mail', { kind: 'role', role: 'sent', label: 'Sent' }));
    bind('g d', 'Go to Drafts', () => this.go('mail', { kind: 'role', role: 'drafts', label: 'Drafts' }));
    bind('g c', 'Go to Calendar', () => this.go('calendar'));
    bind('g r', 'Go to Review queue', () => this.go('review'));
    bind('g ,', 'Go to Settings', () => this.go('settings'));
    bind('mod+k', 'Command palette', () => this.palette());
    bind('?', 'Keyboard shortcuts', () => showShortcuts());
  }

  private palette() {
    const actions: { label: string; run: () => void }[] = [
      { label: 'Compose new message', run: () => openComposer({}) },
      ...UNIFIED.map((u) => ({ label: `Go to ${u.label}`, run: () => this.go('mail', { kind: 'role', role: u.role, label: u.label }) })),
      ...store.state.mailboxes.filter((m) => m.role === 'custom').map((m) => ({ label: `Go to ${m.path}`, run: () => this.go('mail', { kind: 'mailbox', mailbox_id: m.id, label: m.path }) })),
      { label: 'Open Calendar', run: () => this.go('calendar') },
      { label: 'Open Review queue', run: () => this.go('review') },
      { label: 'Open Settings', run: () => this.go('settings') },
      { label: 'Sync all accounts now', run: () => void api.syncNow(null) },
      { label: 'Toggle dark mode', run: () => toggleTheme() },
      { label: 'Keyboard shortcuts', run: () => showShortcuts() },
    ];
    const input = h('input.palette-input', { type: 'text', placeholder: 'Type a command or search mail…', 'aria-label': 'Command' });
    const list = h('div.palette-list', { role: 'listbox' });
    let idx = 0;
    let filtered = actions;
    const render = () => {
      const q = input.value.toLowerCase().trim();
      filtered = actions.filter((a) => a.label.toLowerCase().includes(q));
      if (q) filtered.push({ label: `Search mail for “${input.value.trim()}”`, run: () => this.go('mail', { kind: 'search', query: input.value.trim(), label: `Search: ${input.value.trim()}` }) });
      idx = Math.min(idx, Math.max(filtered.length - 1, 0));
      replace(list, filtered.map((a, i) => h(`button.palette-item${i === idx ? '.active' : ''}`, { role: 'option', on: { click: () => { m.close(); a.run(); } } }, a.label)));
    };
    input.addEventListener('input', () => { idx = 0; render(); });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') idx = Math.min(idx + 1, filtered.length - 1);
      else if (e.key === 'ArrowUp') idx = Math.max(idx - 1, 0);
      else if (e.key === 'Enter') {
        m.close();
        filtered[idx]?.run();
        return;
      } else return;
      e.preventDefault();
      render();
    });
    const m = modal('Commands', h('div.palette', input, list));
    m.root.classList.add('palette-modal');
    render();
  }
}

function toggleTheme() {
  const s = store.state.settings;
  if (!s) return;
  const dark = s.theme === 'dark' || (s.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const next = { ...s, theme: dark ? 'light' : 'dark' };
  applyTheme(next.theme);
  void api.updateSettings(next).then((settings) => store.set({ settings })).catch(errorToast);
}

function showShortcuts() {
  const seen = new Set<string>();
  const rows = shortcutList().filter((s) => {
    const k = `${s.key}|${s.desc}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return !['ArrowDown', 'ArrowUp', 'Backspace', 'o'].includes(s.key);
  });
  modal('Keyboard shortcuts', h('table.shortcuts', h('tbody', rows.map((r) => h('tr', h('td', h('kbd', r.key.replace('mod', navigator.platform.includes('Mac') ? '⌘' : 'Ctrl'))), h('td', r.desc), h('td.muted.small', r.scope))))));
}

async function boot() {
  installShortcuts();
  if (transport.kind === 'tauri' && /Mac/.test(navigator.platform)) {
    // Overlay title bar: leave room for the window controls and make the strip draggable.
    document.body.classList.add('tauri-mac');
    document.body.appendChild(h('div.titlebar-drag', { 'data-tauri-drag-region': '' }));
  }
  try {
    await store.init();
  } catch (e) {
    document.body.append(h('div.fatal', h('h1', 'Can’t reach Satchel Mail'), h('p', e instanceof Error ? e.message : String(e)), h('button.primary', { on: { click: () => location.reload() } }, 'Retry')));
    return;
  }
  new Shell();
  if (!store.state.accounts.length) store.set({ view: 'settings' });
}

void boot();
