// Settings: accounts, AI agents, AI folder, privacy, appearance, activity.

import { api } from '../api';
import { h, icon, replace } from '../lib/dom';
import { fullDate, relative } from '../lib/format';
import { confirmDialog, errorToast, modal, toast } from '../lib/ui';
import { applyTheme, store } from '../state';
import type { AccountConfig, Agent, AgentPolicy, Credential, ProviderKind, Security, Settings } from '../types';
import { openExternal } from './mail';
import { transport } from '../transport';

type Section = 'accounts' | 'agents' | 'mirror' | 'privacy' | 'general' | 'activity';

interface Preset {
  id: string;
  label: string;
  kind: 'imap' | 'jmap' | 'oauth';
  provider?: ProviderKind;
  imap?: { host: string; port: number; security: Security; smtp_host: string; smtp_port: number; smtp_security: Security; caldav_url?: string };
  jmap?: { session_url: string; caldav_url?: string };
  help?: string;
}

const PRESETS: Preset[] = [
  { id: 'gmail', label: 'Google (Gmail)', kind: 'oauth', provider: 'gmail', help: 'Sign in with Google in your browser. Mail, labels and Google Calendar.' },
  { id: 'microsoft', label: 'Microsoft 365 / Outlook', kind: 'oauth', provider: 'graph', help: 'Sign in with Microsoft in your browser. Mail, folders and Outlook calendar.' },
  { id: 'fastmail', label: 'Fastmail', kind: 'jmap', jmap: { session_url: 'https://api.fastmail.com/jmap/session', caldav_url: 'https://caldav.fastmail.com/' }, help: 'Use an app password from Fastmail → Settings → Privacy & Security → Integrations.' },
  { id: 'icloud', label: 'iCloud', kind: 'imap', imap: { host: 'imap.mail.me.com', port: 993, security: 'tls', smtp_host: 'smtp.mail.me.com', smtp_port: 587, smtp_security: 'start_tls', caldav_url: 'https://caldav.icloud.com/' }, help: 'Use an app-specific password from appleid.apple.com.' },
  { id: 'imap', label: 'Other (IMAP/SMTP)', kind: 'imap', imap: { host: '', port: 993, security: 'tls', smtp_host: '', smtp_port: 465, smtp_security: 'tls' } },
  { id: 'jmap', label: 'Other (JMAP)', kind: 'jmap', jmap: { session_url: '' } },
];

const field = (label: string, input: HTMLElement, hint?: string) => h('label.form-field', h('span.form-label', label), input, hint && h('span.form-hint', hint));

export class SettingsView {
  root = h('div.settings-view');
  private section: Section = 'accounts';

  constructor() {
    store.subscribe((_, changed) => {
      if (store.state.view === 'settings' && (changed.has('accounts') || changed.has('settings'))) this.render();
    });
  }

  show() {
    this.render();
  }

  private render() {
    const nav: [Section, string, string][] = [
      ['accounts', 'Accounts', 'user'],
      ['agents', 'AI agents', 'sparkle'],
      ['mirror', 'AI folder', 'folder'],
      ['privacy', 'Privacy', 'shield'],
      ['general', 'General', 'settings'],
      ['activity', 'Activity', 'clock'],
    ];
    const content = h('div.settings-content');
    replace(this.root,
      h('nav.settings-nav', nav.map(([s, l, i]) => h(`button.nav-item${this.section === s ? '.active' : ''}`, { on: { click: () => { this.section = s; this.render(); } } }, icon(i), l))),
      content);
    const r = { accounts: () => this.accounts(content), agents: () => this.agents(content), mirror: () => this.mirror(content), privacy: () => this.privacy(content), general: () => this.general(content), activity: () => this.activity(content) };
    void r[this.section]();
  }

  // ------------------------------------------------------------ accounts

  private accounts(el: HTMLElement) {
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
            a.sync.error && h('div.small.error', a.sync.error)),
          h('button', { on: { click: () => void api.syncNow(a.id).then(() => toast('Syncing…')) } }, 'Sync now'),
          h('button.danger', {
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
      h('div.preset-grid', PRESETS.map((p) => h('button.preset', { on: { click: () => this.addAccount(p) } }, h('strong', p.label), p.help && h('span.small.muted', p.help)))),
    );
  }

  private addAccount(p: Preset) {
    if (p.kind === 'oauth') return this.oauth(p);
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
      if (p.kind === 'jmap') return { kind: 'jmap', ...base, session_url: session.value.trim(), caldav_url: caldav.value.trim() || null, allow_invalid_certs: insecure.checked };
      return {
        kind: 'imap', ...base,
        imap_host: host.value.trim(), imap_port: Number(port.value), imap_security: sec.value as Security,
        smtp_host: shost.value.trim() || host.value.trim(), smtp_port: Number(sport.value), smtp_security: ssec.value as Security,
        caldav_url: caldav.value.trim() || null, allow_invalid_certs: insecure.checked,
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
        h('label.row.inline', insecure, 'Allow self-signed certificates (local test servers only)')),
      status), { footer: h('div.row.end', h('button', { on: { click: () => m.close() } }, 'Cancel'), btn) });
  }

  private async oauth(p: Preset) {
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

  // ------------------------------------------------------------ agents

  private async agents(el: HTMLElement) {
    let agents: Agent[] = [];
    let info: Awaited<ReturnType<typeof api.info>> | null = null;
    try {
      [agents, info] = await Promise.all([api.listAgents(), api.info()]);
    } catch (e) {
      errorToast(e);
    }
    const active = agents.filter((a) => !a.revoked);
    replace(el,
      h('h1', 'AI agents'),
      h('p.muted', 'Agents connect over MCP (or the AI folder) to read your mail and calendars and propose changes. Every proposal waits in Review until you approve it. Agents can never send, delete or change settings.'),
      h('div.card-list', active.map((a) =>
        h('div.account-card',
          icon('sparkle'),
          h('div.grow',
            h('strong', a.name),
            h('div.small.muted', `Created ${relative(a.created_at)} · last used ${relative(a.last_used)}`),
            h('div.small.muted', policyLabel(a.policy))),
          h('button', { on: { click: () => this.editAgent(a) } }, 'Edit access'),
          h('button.danger', { on: { click: async () => { if (await confirmDialog('Revoke agent?', `${a.name} will lose access immediately.`, 'Revoke', true)) { await api.revokeAgent(a.id); this.render(); } } } }, 'Revoke')))),
      h('button.primary', { on: { click: () => this.editAgent(null, info?.data_dir) } }, icon('plus', 16), 'Connect an agent'),
    );
  }

  private editAgent(existing: Agent | null, dataDir?: string) {
    const pol: AgentPolicy = existing?.policy ?? { account_ids: null, since_days: 365, allow_attachments: false, allow_calendar: true, max_proposals_per_hour: 60 };
    const name = h('input', { type: 'text', value: existing?.name ?? 'Claude', placeholder: 'e.g. Claude Code' });
    const acctBoxes = store.state.accounts.map((a) => ({ a, box: h('input', { type: 'checkbox', checked: !pol.account_ids || pol.account_ids.includes(a.id) }) }));
    const since = h('select', [['30', 'Last 30 days'], ['90', 'Last 90 days'], ['365', 'Last year'], ['', 'Everything']].map(([v, l]) => h('option', { value: v, selected: String(pol.since_days ?? '') === v }, l)));
    const att = h('input', { type: 'checkbox', checked: pol.allow_attachments });
    const cal = h('input', { type: 'checkbox', checked: pol.allow_calendar });
    const rate = h('input', { type: 'number', value: String(pol.max_proposals_per_hour), min: '1', max: '1000' });
    const save = async () => {
      const ids = acctBoxes.filter((x) => x.box.checked).map((x) => x.a.id);
      const policy: AgentPolicy = {
        account_ids: ids.length === store.state.accounts.length ? null : ids,
        since_days: since.value ? Number(since.value) : null,
        allow_attachments: att.checked,
        allow_calendar: cal.checked,
        max_proposals_per_hour: Number(rate.value) || 60,
      };
      try {
        if (existing) {
          await api.updateAgent(existing.id, name.value.trim() || existing.name, policy);
          m.close();
          this.render();
          return;
        }
        const r = await api.createAgent(name.value.trim() || 'Agent', policy);
        m.close();
        void this.showToken(r.token, dataDir);
        this.render();
      } catch (e) {
        errorToast(e);
      }
    };
    const m = modal(existing ? `Access for ${existing.name}` : 'Connect an AI agent', h('div.form',
      field('Name', name),
      h('fieldset', h('legend', 'Accounts it can read'), acctBoxes.map(({ a, box }) => h('label.row.inline', box, a.email))),
      field('Mail history', since),
      h('label.row.inline', cal, 'Can read calendars and propose events'),
      h('label.row.inline', att, 'Can read binary attachments (text attachments are always readable)'),
      field('Max proposals per hour', rate)), {
      footer: h('div.row.end', h('button', { on: { click: () => m.close() } }, 'Cancel'), h('button.primary', { on: { click: () => void save() } }, existing ? 'Save' : 'Create token')),
    });
  }

  private async showToken(token: string, dataDir?: string) {
    // The desktop app bundles the CLI; use its absolute path so it works without PATH changes.
    const cli = transport.kind === 'tauri' ? ((await transport.invoke<string | null>('cli_path').catch(() => null)) ?? 'satchel') : 'satchel';
    const quoted = cli.includes(' ') ? `"${cli}"` : cli;
    const env = dataDir && transport.kind !== 'tauri' ? ` --env SATCHEL_DATA_DIR="${dataDir}"` : '';
    const claude = `claude mcp add satchel --env SATCHEL_AGENT_TOKEN=${token}${env} -- ${quoted} mcp`;
    const json = JSON.stringify({ mcpServers: { satchel: { command: cli, args: ['mcp'], env: { SATCHEL_AGENT_TOKEN: token, ...(env && dataDir ? { SATCHEL_DATA_DIR: dataDir } : {}) } } } }, null, 2);
    const copy = (s: string) => () => void navigator.clipboard.writeText(s).then(() => toast('Copied'));
    modal('Agent token created', h('div.form',
      h('p', 'Copy this token now — it won’t be shown again.'),
      h('div.copy-row', h('code.token', token), h('button', { on: { click: copy(token) } }, 'Copy')),
      h('h3', 'Claude Code'),
      h('div.copy-row', h('pre.snippet', claude), h('button', { on: { click: copy(claude) } }, 'Copy')),
      h('h3', 'Claude Desktop / other MCP clients'),
      h('div.copy-row', h('pre.snippet', json), h('button', { on: { click: copy(json) } }, 'Copy')),
      h('p.small.muted', 'Agents without MCP can read the AI folder and drop proposal files into its proposals/inbox folder (see README.md there).')), { wide: true });
  }

  // ------------------------------------------------------------ mirror

  private async mirror(el: HTMLElement) {
    const s = store.state.settings;
    if (!s) return;
    const info = await api.info().catch(() => null);
    const mi = s.mirror;
    const enabled = h('input', { type: 'checkbox', checked: mi.enabled });
    const root = h('input', { type: 'text', value: mi.root ?? '', placeholder: info?.mirror_root ?? '~/Satchel' });
    const since = h('select', [['30', 'Last 30 days'], ['90', 'Last 90 days'], ['365', 'Last year'], ['', 'Everything']].map(([v, l]) => h('option', { value: v, selected: String(mi.since_days ?? '') === v }, l)));
    const eml = h('input', { type: 'checkbox', checked: mi.include_eml });
    const atts = h('input', { type: 'checkbox', checked: mi.include_attachments });
    const acctBoxes = store.state.accounts.map((a) => ({ a, box: h('input', { type: 'checkbox', checked: !mi.account_ids || mi.account_ids.includes(a.id) }) }));
    const save = async () => {
      const ids = acctBoxes.filter((x) => x.box.checked).map((x) => x.a.id);
      await this.saveSettings({ ...s, mirror: { enabled: enabled.checked, root: root.value.trim() || null, since_days: since.value ? Number(since.value) : null, include_eml: eml.checked, include_attachments: atts.checked, account_ids: ids.length === store.state.accounts.length ? null : ids } });
      toast('AI folder settings saved');
    };
    replace(el,
      h('h1', 'AI folder'),
      h('p.muted', `Satchel keeps a plain-text copy of your mail and calendar in a folder any AI tool can read: ${info?.mirror_root ?? '~/Satchel'}. It is one-way — editing these files changes nothing.`),
      h('div.form',
        h('label.row.inline', enabled, 'Keep the AI folder up to date'),
        field('Location', root, 'Leave empty for the default'),
        field('Include mail from', since),
        h('fieldset', h('legend', 'Accounts'), acctBoxes.map(({ a, box }) => h('label.row.inline', box, a.email))),
        h('label.row.inline', eml, 'Include raw .eml files'),
        h('label.row.inline', atts, 'Include attachments (up to 25 MB each)'),
        h('div.row', h('button.primary', { on: { click: () => void save() } }, 'Save'))));
  }

  // ------------------------------------------------------------ privacy

  private privacy(el: HTMLElement) {
    const s = store.state.settings;
    if (!s) return;
    const aiDrafts = h('input', { type: 'checkbox', checked: s.sync_ai_drafts_to_server, on: { change: () => void this.saveSettings({ ...s, sync_ai_drafts_to_server: aiDrafts.checked }) } });
    replace(el,
      h('h1', 'Privacy'),
      h('h2', 'Remote images'),
      h('p.muted', 'Remote images can tell senders when you open their mail. They are blocked unless you allow a sender.'),
      s.remote_image_senders.length
        ? h('ul.plain-list', s.remote_image_senders.map((snd) => h('li', snd, h('button.link', { on: { click: () => void this.saveSettings({ ...s, remote_image_senders: s.remote_image_senders.filter((x) => x !== snd) }) } }, 'Remove'))))
        : h('p.small.muted', 'No senders allowed.'),
      h('h2', 'AI drafts'),
      h('label.row.inline', aiDrafts, 'Copy approved AI drafts to the server Drafts folder (so they appear on your phone). They are never sent automatically.'));
  }

  // ------------------------------------------------------------ general

  private general(el: HTMLElement) {
    const s = store.state.settings;
    if (!s) return;
    const theme = h('select', { on: { change: () => { applyTheme(theme.value); void this.saveSettings({ ...s, theme: theme.value }); } } },
      [['system', 'Match system'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => h('option', { value: v, selected: s.theme === v }, l)));
    const days = h('input', { type: 'number', min: '7', max: '3650', value: String(s.initial_sync_days) });
    const poll = h('input', { type: 'number', min: '15', max: '3600', value: String(s.poll_interval_secs) });
    replace(el,
      h('h1', 'General'),
      h('div.form',
        field('Appearance', theme),
        field('Download full messages from the last (days)', days, 'Older mail syncs headers first; bodies download when opened.'),
        field('Check for new mail every (seconds)', poll, 'Accounts with push (IMAP IDLE, JMAP) update instantly.'),
        h('div.row', h('button.primary', { on: { click: () => void this.saveSettings({ ...s, initial_sync_days: Number(days.value) || 90, poll_interval_secs: Number(poll.value) || 120 }).then(() => toast('Saved')) } }, 'Save'))));
  }

  // ------------------------------------------------------------ activity

  private async activity(el: HTMLElement) {
    try {
      const [audit, outbox] = await Promise.all([api.listAudit(200), api.listOutbox()]);
      const failed = outbox.filter((o) => o.status === 'failed' || o.attempts > 0);
      replace(el,
        h('h1', 'Activity'),
        failed.length > 0 && h('section',
          h('h2', 'Outbox problems'),
          h('ul.plain-list', failed.map((o) => h('li', h('strong', o.kind), ` · ${o.status} · ${o.attempts} attempts`, o.last_error && h('div.small.error', o.last_error), h('button.link', { on: { click: async () => { await api.retryOutbox(o.id); toast('Retrying'); this.render(); } } }, 'Retry'))))),
        h('h2', 'Audit log'),
        h('p.muted.small', 'Every send, approval and agent proposal is recorded here.'),
        h('table.audit', h('tbody', audit.map((a) => h('tr', h('td.small.muted', { title: fullDate(a.at) }, relative(a.at)), h('td', a.actor), h('td', a.action.replace(/_/g, ' ')), h('td.small.muted', summarize(a.detail)))))));
    } catch (e) {
      errorToast(e);
    }
  }

  private async saveSettings(next: Settings) {
    try {
      const saved = await api.updateSettings(next);
      store.set({ settings: saved });
    } catch (e) {
      errorToast(e);
    }
  }
}

function secSelect(v: Security) {
  return h('select', [['tls', 'SSL/TLS'], ['start_tls', 'STARTTLS'], ['none', 'None']].map(([val, l]) => h('option', { value: val, selected: v === val }, l)));
}

function kindLabel(k: ProviderKind) {
  return { imap: 'IMAP', gmail: 'Google', graph: 'Microsoft 365', jmap: 'JMAP' }[k];
}

function policyLabel(p: AgentPolicy) {
  const accts = p.account_ids ? `${p.account_ids.length} account(s)` : 'all accounts';
  const hist = p.since_days ? `last ${p.since_days} days` : 'all history';
  return `${accts} · ${hist}${p.allow_calendar ? ' · calendars' : ''}${p.allow_attachments ? ' · attachments' : ''}`;
}

function summarize(d: Record<string, unknown>) {
  return Object.entries(d)
    .filter(([, v]) => v !== null && v !== undefined && typeof v !== 'object')
    .map(([k, v]) => `${k}: ${String(v)}`)
    .join(' · ')
    .slice(0, 160);
}
