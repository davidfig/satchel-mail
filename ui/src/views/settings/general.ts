// Settings → AI folder, General and Activity.

import { api } from '../../api';
import { h, replace } from '../../lib/dom';
import { fullDate, relative } from '../../lib/format';
import { errorToast, toast } from '../../lib/ui';
import { applyTheme, store } from '../../state';
import { transport } from '../../transport';
import { field } from './form';
import type { SettingsView } from './index';

export async function mirror(view: SettingsView, el: HTMLElement) {
  const s = store.state.settings;
  if (!s) return;
  const info = await api.info().catch(() => null);
  const mi = s.mirror;
  const enabled = h('input', { type: 'checkbox', checked: mi.enabled });
  const root = h('input', { type: 'text', value: mi.root ?? '', placeholder: info?.mirror_root ?? '~/Satchel' });
  const since = h('select', [['30', 'Last 30 days'], ['90', 'Last 90 days'], ['365', 'Last year'], ['', 'Everything']].map(([v, l]) => h('option', { value: v, selected: String(mi.since_days ?? '') === v }, l)));
  const eml = h('input', { type: 'checkbox', checked: mi.include_eml });
  const atts = h('input', { type: 'checkbox', checked: mi.include_attachments });
  const proposals = h('input', { type: 'checkbox', checked: mi.accept_proposals });
  const calendar = h('input', { type: 'checkbox', checked: mi.include_calendar });
  const contacts = h('input', { type: 'checkbox', checked: mi.include_contacts });
  const acctBoxes = store.state.accounts.map((a) => ({ a, box: h('input', { type: 'checkbox', checked: !mi.account_ids || mi.account_ids.includes(a.id) }) }));
  // A folder picked from the system dialog gets a Satchel folder inside it,
  // so picking Documents doesn't mix mail into the user's own files.
  const choose = async () => {
    const picked = await transport.host<string | null>('choose_folder', { title: 'Choose where to keep the AI folder', start: info?.mirror_root ?? null }).catch(errorToast);
    const sep = picked?.includes('\\') ? '\\' : '/';
    if (picked) root.value = /[\\/]Satchel$/.test(picked) ? picked : `${picked.replace(/[\\/]$/, '')}${sep}Satchel`;
  };
  const save = async () => {
    const ids = acctBoxes.filter((x) => x.box.checked).map((x) => x.a.id);
    const saved = await view.saveSettings({ ...s, mirror: { enabled: enabled.checked, root: root.value.trim() || null, since_days: since.value ? Number(since.value) : null, include_eml: eml.checked, include_attachments: atts.checked, accept_proposals: proposals.checked, include_calendar: calendar.checked, include_contacts: contacts.checked, account_ids: ids.length === store.state.accounts.length ? null : ids } });
    if (!saved) return;
    const moved = (await api.info().catch(() => null))?.mirror_root !== info?.mirror_root;
    toast(moved ? 'Moved the AI folder. Point your AI tools at the new location.' : 'AI folder settings saved', { quiet: !moved });
  };
  replace(el,
    h('h1', 'AI folder'),
    h('p.muted', `Satchel keeps a plain-text copy of your mail and calendar in a folder you can give any AI tool: ${info?.mirror_root ?? '~/Satchel'}. Editing the copies changes nothing; to act, the AI writes a proposal file, which waits in AI Proposals for you.`),
    h('div.form',
      h('label.row.inline', enabled, 'Keep the AI folder up to date'),
      field('Location', transport.kind === 'tauri' ? h('div.row.picker', root, h('button', { type: 'button', title: 'Pick a folder; Satchel keeps its files in a Satchel folder inside it', on: { click: () => void choose() } }, 'Choose…')) : root,
        'Leave empty for ~/Satchel. Changing it moves the folder, with any proposals waiting in it, when you save.'),
      field('Include mail from', since),
      h('fieldset', h('legend', 'Accounts'), acctBoxes.map(({ a, box }) => h('label.row.inline', box, a.email))),
      h('label.row.inline', eml, 'Include raw .eml files'),
      h('label.row.inline', atts, 'Include attachments (up to 25 MB each)'),
      h('label.row.inline', calendar, 'Include the calendar'),
      h('label.row.inline', contacts, 'Include contacts (AI tools can propose edits to them)'),
      h('label.row.inline', proposals, 'Let AI tools propose changes by writing files to proposals/inbox'),
      h('div.row', h('button.primary', { on: { click: () => void save() } }, 'Save'))));
}

export function general(view: SettingsView, el: HTMLElement) {
  const s = store.state.settings;
  if (!s) return;
  const theme = h('select', { on: { change: () => { applyTheme(theme.value); void view.saveSettings({ ...s, theme: theme.value }); } } },
    [['system', 'Match system'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => h('option', { value: v, selected: s.theme === v }, l)));
  const days = h('input', { type: 'number', min: '7', max: '3650', value: String(s.initial_sync_days) });
  const poll = h('input', { type: 'number', min: '15', max: '3600', value: String(s.poll_interval_secs) });
  const option = (cur: number | null) => ([v, l]: string[]) => h('option', { value: v, selected: String(cur ?? '') === v }, l);
  const background = h('select', [['365', 'Last year'], ['1825', 'Last 5 years'], ['', 'All mail']].map(option(s.download_days)));
  const limit = h('select', [['', 'No limit'], ['1024', '1 GB'], ['2048', '2 GB'], ['5120', '5 GB'], ['10240', '10 GB'], ['25600', '25 GB'], ['51200', '50 GB']].map(option(s.storage_limit_mb)));
  const notify = h('input', {
    type: 'checkbox',
    checked: s.notify_new_mail,
    on: {
      change: async () => {
        if (notify.checked && !(await transport.canNotify(true))) toast('Notifications are turned off for Satchel. Allow them in your system or browser settings.', { kind: 'error', ms: 7000 });
        void view.saveSettings({ ...(store.state.settings ?? s), notify_new_mail: notify.checked });
      },
    },
  });
  replace(el,
    h('h1', 'General'),
    h('div.form',
      field('Appearance', theme),
      field('Download messages from the last (days) first', days, 'These come first; older mail lists right away and its content follows in the background.'),
      field('Keep downloading in the background', background, 'Older mail downloads when you open it. Attachments always download only when you open them.'),
      field('Storage limit per account', limit, 'Over the limit, the content of the oldest mail is removed (it stays listed and searchable, and downloads again when opened).'),
      field('Check for new mail every (seconds)', poll, 'Accounts with push (IMAP IDLE, JMAP) update instantly.'),
      h('label.row.inline', notify, 'Show a notification when new mail arrives in an inbox (only while you’re in another app)'),
      h('div.row', h('button.primary', { on: { click: () => void view.saveSettings({ ...(store.state.settings ?? s), initial_sync_days: Number(days.value) || 90, download_days: background.value ? Number(background.value) : null, storage_limit_mb: limit.value ? Number(limit.value) : null, poll_interval_secs: Number(poll.value) || 120 }).then(() => toast('Saved', { quiet: true })) } }, 'Save'))));
}

export async function activity(view: SettingsView, el: HTMLElement) {
  try {
    const [audit, outbox] = await Promise.all([api.listAudit(200), api.listOutbox()]);
    const failed = outbox.filter((o) => o.status === 'failed' || o.attempts > 0);
    replace(el,
      h('h1', 'Activity'),
      failed.length > 0 && h('section',
        h('h2', 'Outbox problems'),
        h('ul.plain-list', failed.map((o) => h('li', h('strong', o.kind), ` · ${o.status} · ${o.attempts} attempts`, o.last_error && h('div.small.error', o.last_error), h('button.link', { title: 'Try this change again now', on: { click: async () => { await api.retryOutbox(o.id); toast('Retrying', { quiet: true }); view.render(); } } }, 'Retry'))))),
      h('h2', 'Audit log'),
      h('p.muted.small', 'Every send, approval and agent proposal is recorded here.'),
      h('table.audit', h('tbody', audit.map((a) => h('tr', h('td.small.muted', { title: fullDate(a.at) }, relative(a.at)), h('td', a.actor), h('td', a.action.replace(/_/g, ' ')), h('td.small.muted', summarize(a.detail)))))));
  } catch (e) {
    errorToast(e);
  }
}

function summarize(d: Record<string, unknown>) {
  return Object.entries(d)
    .filter(([, v]) => v !== null && v !== undefined && typeof v !== 'object')
    .map(([k, v]) => `${k}: ${String(v)}`)
    .join(' · ')
    .slice(0, 160);
}
