// Settings → AI folder, General and Activity.

import { api } from '../../api';
import { debounce, h, patch } from '../../lib/dom';
import { fullDate, relative } from '../../lib/format';
import { errorToast, toast } from '../../lib/ui';
import { applyTheme, store } from '../../state';
import { transport } from '../../transport';
import { field } from './form';
import type { MirrorSettings, Settings } from '../../types';
import type { SettingsView } from './index';

// Every AI folder change rewrites the whole folder, so changes made close
// together are saved as one.
const saveMirrorSoon = debounce((view: SettingsView, mirror: MirrorSettings) => {
  const s = store.state.settings;
  if (s) void view.saveSettings({ ...s, mirror });
}, 500);

export async function mirror(view: SettingsView, el: HTMLElement) {
  const s = store.state.settings;
  if (!s) return;
  const info = await api.info().catch(() => null);
  // Handlers edit this copy, not captured inputs: a redraw patches new
  // handlers onto the old nodes (0046).
  const draft: MirrorSettings = { ...s.mirror };
  const set = (change: Partial<MirrorSettings>) => {
    Object.assign(draft, change);
    saveMirrorSoon(view, { ...draft, root: store.state.settings?.mirror.root ?? draft.root });
  };
  const box = (key: 'enabled' | 'include_eml' | 'include_attachments' | 'accept_proposals' | 'include_calendar' | 'include_contacts', label: string) =>
    h('label.row.inline', h('input', { type: 'checkbox', checked: draft[key], on: { change: (e: Event) => set({ [key]: (e.currentTarget as HTMLInputElement).checked }) } }), label);
  const since = h('select', { on: { change: (e: Event) => { const v = (e.currentTarget as HTMLSelectElement).value; set({ since_days: v ? Number(v) : null }); } } },
    [['30', 'Last 30 days'], ['90', 'Last 90 days'], ['365', 'Last year'], ['', 'Everything']].map(([v, l]) => h('option', { value: v, selected: String(draft.since_days ?? '') === v }, l)));
  const accounts = (e: Event) => {
    const boxes = [...(e.currentTarget as HTMLElement).closest('fieldset')!.querySelectorAll('input')];
    const ids = boxes.filter((b) => b.checked).map((b) => b.value);
    set({ account_ids: ids.length === boxes.length ? null : ids });
  };
  const locationInput = (e: Event) => (e.currentTarget as HTMLElement).closest('.picker')!.querySelector('input')!;
  // A folder picked from the system dialog gets a Satchel folder inside it,
  // so picking Documents doesn't mix mail into the user's own files.
  const choose = async (e: Event) => {
    const input = locationInput(e);
    const picked = await transport.host<string | null>('choose_folder', { title: 'Choose where to keep the AI folder', start: info?.mirror_root ?? null }).catch(errorToast);
    const sep = picked?.includes('\\') ? '\\' : '/';
    if (picked) input.value = /[\\/]Satchel$/.test(picked) ? picked : `${picked.replace(/[\\/]$/, '')}${sep}Satchel`;
  };
  // Moving is the one change that waits to be asked for: it carries the
  // whole folder, proposals and all, to the new place.
  const move = async (e: Event) => {
    const root = locationInput(e).value.trim() || null;
    const cur = store.state.settings;
    if (!cur || root === cur.mirror.root) return;
    if (!(await view.saveSettings({ ...cur, mirror: { ...cur.mirror, root } }))) return;
    const moved = (await api.info().catch(() => null))?.mirror_root !== info?.mirror_root;
    if (moved) toast('Moved the AI folder. Point your AI tools at the new location.');
  };
  const root = h('input', { type: 'text', value: draft.root ?? '', placeholder: info?.mirror_root ?? '~/Satchel', on: { keydown: (e: Event) => { if ((e as KeyboardEvent).key === 'Enter') void move(e); } } });
  patch(el,
    h('h1', 'AI folder'),
    h('p.muted', `Satchel keeps a plain-text copy of your mail and calendar in a folder you can give any AI tool: ${info?.mirror_root ?? '~/Satchel'}. Editing the copies changes nothing; to act, the AI writes a proposal file, which waits in AI Proposals for you.`),
    h('div.form',
      box('enabled', 'Keep the AI folder up to date'),
      field('Location', h('div.row.picker', root,
        transport.kind === 'tauri' && h('button', { type: 'button', title: 'Pick a folder; Satchel keeps its files in a Satchel folder inside it', on: { click: (e: Event) => void choose(e) } }, 'Choose…'),
        h('button', { type: 'button', title: 'Move the AI folder here (Enter)', on: { click: (e: Event) => void move(e) } }, 'Move')),
        'Leave empty for ~/Satchel. Moving takes the folder, with any proposals waiting in it, to the new place.'),
      field('Include mail from', since),
      h('fieldset', h('legend', 'Accounts'), store.state.accounts.map((a) => h('label.row.inline', h('input', { type: 'checkbox', value: a.id, checked: !draft.account_ids || draft.account_ids.includes(a.id), on: { change: accounts } }), a.email))),
      box('include_eml', 'Include raw .eml files'),
      box('include_attachments', 'Include attachments (up to 25 MB each)'),
      box('include_calendar', 'Include the calendar'),
      box('include_contacts', 'Include contacts (AI tools can propose edits to them)'),
      box('accept_proposals', 'Let AI tools propose changes by writing files to proposals/inbox')));
}

export function general(view: SettingsView, el: HTMLElement) {
  const s = store.state.settings;
  if (!s) return;
  // Each change saves on its own, read from the event: a redraw patches new
  // handlers onto the old nodes (0046).
  const save = (change: Partial<Settings>) => {
    const cur = store.state.settings;
    if (cur) void view.saveSettings({ ...cur, ...change });
  };
  const value = (e: Event) => (e.currentTarget as HTMLInputElement | HTMLSelectElement).value;
  const optional = (v: string) => (v ? Number(v) : null);
  const theme = h('select', { on: { change: (e: Event) => { applyTheme(value(e)); save({ theme: value(e) }); } } },
    [['system', 'Match system'], ['light', 'Light'], ['dark', 'Dark']].map(([v, l]) => h('option', { value: v, selected: s.theme === v }, l)));
  const days = h('input', { type: 'number', min: '7', max: '3650', value: String(s.initial_sync_days), on: { change: (e: Event) => save({ initial_sync_days: Number(value(e)) || 90 }) } });
  const poll = h('input', { type: 'number', min: '15', max: '3600', value: String(s.poll_interval_secs), on: { change: (e: Event) => save({ poll_interval_secs: Number(value(e)) || 120 }) } });
  const option = (cur: number | null) => ([v, l]: string[]) => h('option', { value: v, selected: String(cur ?? '') === v }, l);
  const background = h('select', { on: { change: (e: Event) => save({ download_days: optional(value(e)) }) } }, [['365', 'Last year'], ['1825', 'Last 5 years'], ['', 'All mail']].map(option(s.download_days)));
  const limit = h('select', { on: { change: (e: Event) => save({ storage_limit_mb: optional(value(e)) }) } }, [['', 'No limit'], ['1024', '1 GB'], ['2048', '2 GB'], ['5120', '5 GB'], ['10240', '10 GB'], ['25600', '25 GB'], ['51200', '50 GB']].map(option(s.storage_limit_mb)));
  const notify = h('input', {
    type: 'checkbox',
    checked: s.notify_new_mail,
    on: {
      change: async (e: Event) => {
        const on = (e.currentTarget as HTMLInputElement).checked;
        if (on && !(await transport.canNotify(true))) toast('Notifications are turned off for Satchel. Allow them in your system or browser settings.', { kind: 'error', ms: 7000 });
        save({ notify_new_mail: on });
      },
    },
  });
  patch(el,
    h('h1', 'General'),
    h('div.form',
      field('Appearance', theme),
      field('Download messages from the last (days) first', days, 'These come first; older mail lists right away and its content follows in the background.'),
      field('Keep downloading in the background', background, 'Older mail downloads when you open it. Attachments always download only when you open them.'),
      field('Storage limit per account', limit, 'Over the limit, the content of the oldest mail is removed (it stays listed and searchable, and downloads again when opened).'),
      field('Check for new mail every (seconds)', poll, 'Accounts with push (IMAP IDLE, JMAP) update instantly.'),
      h('label.row.inline', notify, 'Show a notification when new mail arrives in an inbox (only while you’re in another app)')));
}

export async function activity(view: SettingsView, el: HTMLElement) {
  try {
    const [audit, outbox] = await Promise.all([api.listAudit(200), api.listOutbox()]);
    const failed = outbox.filter((o) => o.status === 'failed' || o.attempts > 0);
    patch(el,
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
