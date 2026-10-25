// Settings → Accounts: the account list and adding or reconnecting accounts.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { relative } from '../../lib/format';
import { confirmDialog, errorToast, modal, toast } from '../../lib/ui';
import { store } from '../../state';
import type { Account, AccountConfig, Credential, ProviderKind, Security } from '../../types';
import { openExternal } from '../mail/message-body';
import { field } from './form';

interface Preset {
  id: string;
  label: string;
  kind: 'imap' | 'jmap' | 'oauth';
  provider?: ProviderKind;
  imap?: { host: string; port: number; security: Security; smtp_host: string; smtp_port: number; smtp_security: Security; caldav_url?: string; carddav_url?: string };
  jmap?: { session_url: string; caldav_url?: string; carddav_url?: string };
  help?: string;
}

const PRESETS: Preset[] = [
  { id: 'gmail', label: 'Google (Gmail)', kind: 'oauth', provider: 'gmail', help: 'Sign in with Google in your browser. Mail, labels and Google Calendar.' },
  { id: 'microsoft', label: 'Microsoft 365 / Outlook', kind: 'oauth', provider: 'graph', help: 'Sign in with Microsoft in your browser. Mail, folders and Outlook calendar.' },
  { id: 'fastmail', label: 'Fastmail', kind: 'jmap', jmap: { session_url: 'https://api.fastmail.com/jmap/session', caldav_url: 'https://caldav.fastmail.com/', carddav_url: 'https://carddav.fastmail.com/' }, help: 'Use an app password from Fastmail → Settings → Privacy & Security → Integrations.' },
  { id: 'icloud', label: 'iCloud', kind: 'imap', imap: { host: 'imap.mail.me.com', port: 993, security: 'tls', smtp_host: 'smtp.mail.me.com', smtp_port: 587, smtp_security: 'start_tls', caldav_url: 'https://caldav.icloud.com/', carddav_url: 'https://contacts.icloud.com/' }, help: 'Use an app-specific password from appleid.apple.com.' },
  { id: 'imap', label: 'Other (IMAP/SMTP)', kind: 'imap', imap: { host: '', port: 993, security: 'tls', smtp_host: '', smtp_port: 465, smtp_security: 'tls' } },
  { id: 'jmap', label: 'Other (JMAP)', kind: 'jmap', jmap: { session_url: '' } },
];

export function accounts(el: HTMLElement) {
  const { accounts } = store.state;
  replace(el,
    h('h1', 'Accounts'),
    !accounts.length && h('div.banner', icon('mail', 16), 'Add your first account to start syncing mail and calendars.'),
    h('div.card-list', accounts.map((a) =>
      h('div.account-card',
        h('span.acct-dot.big', { style: { background: a.color } }),
        h('div.grow',
          h('div', h('strong', a.display_name || a.email), h('span.muted', ` ${a.email}`)),
          h('div.small.muted', `${kindLabel(a.kind)} · ${a.sync.state === 'syncing' ? a.sync.detail ?? 'Syncing…' : `Last synced ${relative(a.sync.last_success)}`}`),
          a.sync.error && h('div.small.error', a.sync.error),
          a.contacts_need_reconnect && h('div.small.muted', 'Contacts don’t sync yet: this sign-in predates contacts access. ',
            h('button.link', { title: 'Sign in again to allow Satchel to sync your contacts', on: { click: () => void reconnect(a) } }, 'Reconnect to sync contacts'))),
        h('button', { title: 'Check this account for new mail now', on: { click: () => void api.syncNow(a.id).then(() => toast('Syncing…')) } }, 'Sync now'),
        h('button.danger', {
          title: 'Remove this account from Satchel; nothing is deleted on the server',
          on: {
            click: async () => {
              if (!(await confirmDialog('Remove account?', `Remove ${a.email}? Local copies of its mail are deleted. Nothing is deleted on the server.`, 'Remove', true))) return;
              try {
                await api.removeAccount(a.id);
                await store.refreshAccounts();
                await store.refreshMailboxes();
              } catch (e) {
                errorToast(e);
              }
            },
          },
        }, 'Remove')))),
    h('h2', 'Add an account'),
    h('div.preset-grid', PRESETS.map((p) => h('button.preset', { on: { click: () => addAccount(p) } }, h('strong', p.label), p.help && h('span.small.muted', p.help)))),
  );
}

export function addAccount(p: Preset) {
  if (p.kind === 'oauth') return oauth(p);
  const email = h('input', { type: 'email', placeholder: 'you@example.com', autocomplete: 'email' });
  const name = h('input', { type: 'text', placeholder: 'Your name' });
  const username = h('input', { type: 'text', placeholder: 'Usually your email address', autocomplete: 'username' });
  const password = h('input', { type: 'password', placeholder: p.id === 'icloud' || p.id === 'fastmail' ? 'App password' : 'Password', autocomplete: 'current-password' });
  const insecure = h('input', { type: 'checkbox' });
  const imap = p.imap;
  const host = h('input', { type: 'text', value: imap?.host ?? '' });
  const port = h('input', { type: 'number', value: String(imap?.port ?? 993) });
  const sec = secSelect(imap?.security ?? 'tls');
  const shost = h('input', { type: 'text', value: imap?.smtp_host ?? '' });
  const sport = h('input', { type: 'number', value: String(imap?.smtp_port ?? 465) });
  const ssec = secSelect(imap?.smtp_security ?? 'tls');
  const caldav = h('input', { type: 'url', value: imap?.caldav_url ?? p.jmap?.caldav_url ?? '', placeholder: 'https://… (optional)' });
  const carddav = h('input', { type: 'url', value: imap?.carddav_url ?? p.jmap?.carddav_url ?? '', placeholder: 'https://… (optional)' });
  const session = h('input', { type: 'url', value: p.jmap?.session_url ?? '', placeholder: 'https://…/jmap/session' });
  const advanced = p.id === 'imap' || p.id === 'jmap';
  const status = h('p.small');
  email.addEventListener('input', () => {
    if (!username.dataset.touched) username.value = email.value;
    if (p.id === 'imap') {
      const domain = email.value.split('@')[1];
      if (domain && !host.dataset.touched) host.value = `imap.${domain}`;
      if (domain && !shost.dataset.touched) shost.value = `smtp.${domain}`;
    }
  });
  for (const i of [username, host, shost]) i.addEventListener('input', () => (i.dataset.touched = '1'));
  const config = (): AccountConfig => {
    const base = { email: email.value.trim(), display_name: name.value.trim(), username: username.value.trim() || email.value.trim() };
    if (p.kind === 'jmap') return { kind: 'jmap', ...base, session_url: session.value.trim(), caldav_url: caldav.value.trim() || null, carddav_url: carddav.value.trim() || null, allow_invalid_certs: insecure.checked };
    return {
      kind: 'imap', ...base,
      imap_host: host.value.trim(), imap_port: Number(port.value), imap_security: sec.value as Security,
      smtp_host: shost.value.trim() || host.value.trim(), smtp_port: Number(sport.value), smtp_security: ssec.value as Security,
      caldav_url: caldav.value.trim() || null, carddav_url: carddav.value.trim() || null, allow_invalid_certs: insecure.checked,
    };
  };
  const cred = (): Credential => ({ type: 'password', password: password.value });
  const add = async () => {
    status.className = 'small';
    status.textContent = 'Connecting…';
    btn.disabled = true;
    try {
      await api.addAccount(config(), cred());
      m.close();
      toast('Account added. Syncing…', { kind: 'success' });
      await store.refreshAccounts();
    } catch (e) {
      status.className = 'small error';
      status.textContent = e instanceof Error ? e.message : String(e);
    } finally {
      btn.disabled = false;
    }
  };
  const btn = h('button.primary', { on: { click: () => void add() } }, 'Add account');
  const m = modal(`Add ${p.label}`, h('div.form',
    p.help && h('p.muted.small', p.help),
    field('Email', email),
    field('Name', name, 'Shown to people you email'),
    field('Password', password),
    h('details', { open: advanced },
      h('summary', 'Server settings'),
      field('Username', username),
      p.kind === 'jmap'
        ? field('JMAP session URL', session)
        : [h('div.row', field('IMAP server', host), field('Port', port), field('Security', sec)), h('div.row', field('SMTP server', shost), field('Port', sport), field('Security', ssec))],
      field('CalDAV URL', caldav, 'For calendars. Leave empty to skip.'),
      field('CardDAV URL', carddav, 'For contacts. Leave empty to look them up on the CalDAV server.'),
      h('label.row.inline', insecure, 'Allow self-signed certificates (local test servers only)')),
    status), { footer: h('div.row.end', h('button', { on: { click: () => m.close() } }, 'Cancel'), btn) });
}

export async function oauth(p: Preset) {
  try {
    const { auth_url } = await api.startOAuth(p.provider!);
    openExternal(auth_url);
    const m = modal(`Sign in to ${p.label}`, h('div',
      h('p', 'Finish signing in in your browser. This window updates when you’re done.'),
      h('p.small.muted', h('a', { href: auth_url, on: { click: (e: MouseEvent) => { e.preventDefault(); openExternal(auth_url); } } }, 'Open the sign-in page again'))));
    const before = store.state.accounts.length;
    const off = store.subscribe((s, changed) => {
      if (changed.has('accounts') && s.accounts.length > before) {
        off();
        m.close();
        toast('Account added. Syncing…', { kind: 'success' });
      }
    });
  } catch (e) {
    errorToast(e);
  }
}

/** Sign in again to an OAuth account (e.g. to grant contacts access). */
export async function reconnect(a: Account) {
  try {
    const { auth_url } = await api.startOAuth(a.kind);
    openExternal(auth_url);
    const m = modal(`Reconnect ${a.email}`, h('div',
      h('p', 'Sign in again in your browser and allow access to contacts. This window updates when you’re done.'),
      h('p.small.muted', h('a', { href: auth_url, on: { click: (e: MouseEvent) => { e.preventDefault(); openExternal(auth_url); } } }, 'Open the sign-in page again'))));
    const off = store.subscribe((s, changed) => {
      if (changed.has('accounts') && !s.accounts.find((x) => x.id === a.id)?.contacts_need_reconnect) {
        off();
        m.close();
        toast('Reconnected. Contacts will sync shortly.', { kind: 'success' });
      }
    });
  } catch (e) {
    errorToast(e);
  }
}

function secSelect(v: Security) {
  return h('select', [['tls', 'SSL/TLS'], ['start_tls', 'STARTTLS'], ['none', 'None']].map(([val, l]) => h('option', { value: val, selected: v === val }, l)));
}

function kindLabel(k: ProviderKind) {
  return { imap: 'IMAP', gmail: 'Google', graph: 'Microsoft 365', jmap: 'JMAP' }[k];
}
