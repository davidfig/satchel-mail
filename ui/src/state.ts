// App-wide state with a minimal subscribe/notify store.

import { api } from './api';
import { transport } from './transport';
import type { Account, CoreEvent, Mailbox, MailboxRole, Settings } from './types';
import { notifyNewMail } from './lib/notify';
import { toast } from './lib/ui';

export type SettingsSection = 'accounts' | 'agents' | 'mirror' | 'privacy' | 'favorites' | 'general' | 'activity';
export type View = 'mail' | 'calendar' | 'contacts' | 'review' | 'settings';

/** What the mail list is showing. */
export type MailSource =
  | { kind: 'role'; role: MailboxRole; label: string }
  | { kind: 'mailbox'; mailbox_id: string; label: string }
  | { kind: 'search'; query: string; label: string }
  | { kind: 'person'; emails: string[]; label: string; filter: PersonFilter; back?: MailSource };

/** Which of a person's conversations to list. */
export type PersonFilter = 'all' | 'from' | 'files';

interface AppState {
  view: View;
  accounts: Account[];
  mailboxes: Mailbox[];
  settings: Settings | null;
  pendingProposals: number;
  source: MailSource;
  selectedThread: string | null;
  /** Contact for the Contacts view to show next. */
  selectedContact: string | null;
  /** Settings section to open next. */
  settingsSection: SettingsSection | null;
  myEmails: Set<string>;
  /** Favorite sender addresses, lowercase (0053). */
  favorites: Set<string>;
}

type Listener = (s: AppState, changed: Set<keyof AppState>) => void;

class Store {
  state: AppState = {
    view: 'mail',
    accounts: [],
    mailboxes: [],
    settings: null,
    pendingProposals: 0,
    source: { kind: 'role', role: 'inbox', label: 'Inbox' },
    selectedThread: null,
    selectedContact: null,
    settingsSection: null,
    myEmails: new Set(),
    favorites: new Set(),
  };
  private listeners = new Set<Listener>();
  private eventListeners = new Set<(e: CoreEvent) => void>();

  set(patch: Partial<AppState>) {
    const changed = new Set(Object.keys(patch) as (keyof AppState)[]);
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l(this.state, changed);
  }

  subscribe(fn: Listener) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  onEvent(fn: (e: CoreEvent) => void) {
    this.eventListeners.add(fn);
    return () => this.eventListeners.delete(fn);
  }

  async refreshAccounts() {
    const accounts = await api.listAccounts();
    this.set({ accounts, myEmails: new Set(accounts.map((a) => a.email.toLowerCase())) });
  }

  async refreshMailboxes() {
    this.set({ mailboxes: await api.listMailboxes(null) });
  }

  async refreshProposals() {
    const p = await api.listProposals('pending');
    const stale = await api.listProposals('stale');
    this.set({ pendingProposals: p.length + stale.length });
  }

  async refreshFavorites() {
    this.set({ favorites: new Set(await api.listFavoriteSenders()) });
  }

  async init() {
    const [settings] = await Promise.all([api.getSettings(), this.refreshAccounts(), this.refreshMailboxes(), this.refreshProposals(), this.refreshFavorites()]);
    this.set({ settings });
    applyTheme(settings.theme);
    const refresh = debounceAsync(async () => {
      await Promise.all([this.refreshMailboxes(), this.refreshAccounts()]);
    }, 400);
    transport.subscribe((e) => {
      switch (e.event) {
        case 'accounts_changed':
          void this.refreshAccounts();
          void this.refreshMailboxes();
          break;
        case 'mailboxes_changed':
        case 'messages_changed':
          refresh();
          break;
        case 'sync_status': {
          const accounts = this.state.accounts.map((a) => (a.id === e.account_id ? { ...a, sync: e.status } : a));
          this.set({ accounts });
          break;
        }
        case 'proposals_changed':
          void this.refreshProposals();
          break;
        case 'favorites_changed':
          void this.refreshFavorites();
          break;
        case 'data_changed':
          refresh();
          void this.refreshProposals();
          void this.refreshFavorites();
          break;
        case 'op_failed':
          toast(`${e.description} failed: ${e.error}`, { kind: 'error', ms: 9000 });
          break;
        case 'notice':
          toast(e.message, { kind: e.level === 'error' ? 'error' : 'info' });
          break;
        case 'new_mail':
          void notifyNewMail(e.messages);
          break;
      }
      for (const l of this.eventListeners) l(e);
    });
  }

  accountColor(id: string) {
    return this.state.accounts.find((a) => a.id === id)?.color ?? 'var(--accent)';
  }

  mailbox(id: string) {
    return this.state.mailboxes.find((m) => m.id === id);
  }
}

function debounceAsync(fn: () => Promise<void>, ms: number) {
  let t: number | undefined;
  let running = false;
  let again = false;
  const go = async () => {
    if (running) {
      again = true;
      return;
    }
    running = true;
    try {
      await fn();
    } finally {
      running = false;
      if (again) {
        again = false;
        void go();
      }
    }
  };
  return () => {
    clearTimeout(t);
    t = window.setTimeout(go, ms);
  };
}

export function applyTheme(theme: string) {
  const root = document.documentElement;
  if (theme === 'light' || theme === 'dark') root.dataset.theme = theme;
  else delete root.dataset.theme;
}

export const store = new Store();
