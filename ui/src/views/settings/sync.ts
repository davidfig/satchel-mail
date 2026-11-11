// Settings → Sync: keep settings in step between computers through a
// shared folder, encrypted with a sync code (0080).

import { api } from '../../api';
import { h, icon, patch } from '../../lib/dom';
import { relative } from '../../lib/format';
import { confirmDialog, copyBlock, errorToast, modal } from '../../lib/ui';
import { store } from '../../state';
import { transport } from '../../transport';
import type { FolderSync } from '../../types';
import { signInOffered } from './accounts';
import { field } from './form';
import type { SettingsView } from './index';

const WHAT = 'Accounts (you sign in once on each computer), AI agents and their access, AI proposals (review them on any computer), smart folders, spam rules, favorite senders and preferences. Mail, passwords and tokens never go to the folder, and what does is encrypted with a sync code only your computers have.';

export async function sync(view: SettingsView, el: HTMLElement) {
  let st: FolderSync | null = null;
  try {
    st = await api.getSync();
  } catch (e) {
    errorToast(e);
  }
  if (!st) return;
  patch(el, h('h1', 'Sync'), ...(st.enabled ? on(view, st) : off(view)));
}

function off(view: SettingsView) {
  const folder = h('input', { type: 'text', placeholder: 'A folder iCloud Drive, Dropbox or your vault keeps in step' });
  const code = h('input', { type: 'text', placeholder: 'XXXX-XXXX-…', autocomplete: 'off', spellcheck: false });
  const folderOf = (e: Event) => ((e.currentTarget as HTMLElement).closest('.form')!.querySelector('input') as HTMLInputElement).value.trim();
  const choose = async (e: Event) => {
    const input = (e.currentTarget as HTMLElement).closest('.picker')!.querySelector('input')!;
    const picked = await transport.host<string | null>('choose_folder', { title: 'Choose a folder your computers share', start: null }).catch(errorToast);
    if (picked) input.value = picked;
  };
  const start = async (e: Event) => {
    const dir = folderOf(e);
    if (!dir) return errorToast(new Error('Choose a folder first.'));
    try {
      const { code } = await api.enableSync(dir);
      showCode(code);
      view.render();
    } catch (err) {
      errorToast(err);
    }
  };
  const join = async (e: Event) => {
    const dir = folderOf(e);
    const typed = ((e.currentTarget as HTMLElement).closest('.form')!.querySelector('input[autocomplete=off]') as HTMLInputElement).value;
    if (!dir || !typed.trim()) return errorToast(new Error('Choose the folder and type the sync code from your other computer.'));
    try {
      await api.joinSync(dir, typed);
      store.set({ settings: await api.getSettings() });
      view.render();
    } catch (err) {
      errorToast(err);
    }
  };
  return [
    h('p.muted', `Keep Satchel’s settings the same on all your computers through a folder that already follows you between them. ${WHAT}`),
    h('div.form',
      field('Shared folder', h('div.row.picker', folder,
        transport.kind === 'tauri' && h('button', { type: 'button', title: 'Pick the shared folder', on: { click: (e: Event) => void choose(e) } }, 'Choose…'))),
      h('h2', 'First computer'),
      h('div.row', h('button.primary', { title: 'Start syncing through this folder and get a sync code for your other computers', on: { click: (e: Event) => void start(e) } }, 'Start syncing')),
      h('h2', 'Another computer'),
      field('Sync code', code, 'Shown on the computer that started syncing.'),
      h('div.row', h('button', { title: 'Join the sync in this folder', on: { click: (e: Event) => void join(e) } }, 'Join')),
      h('p.small.muted', 'Joining takes the settings already in the folder; anything only this computer has is added to them.')),
  ];
}

function on(view: SettingsView, st: FolderSync) {
  const now = async () => {
    try {
      await api.syncSettingsNow();
      view.render();
    } catch (e) {
      errorToast(e);
    }
  };
  const stop = async () => {
    if (!(await confirmDialog('Stop syncing?', 'This computer keeps its settings but stops sharing them. To sync again later you’ll need the sync code.', 'Stop syncing', true))) return;
    try {
      await api.disableSync();
      view.render();
    } catch (e) {
      errorToast(e);
    }
  };
  return [
    h('p.muted', WHAT),
    st.error && h('div.banner.warn', icon('warning', 16), st.error),
    h('div.form',
      field('Shared folder', h('code', st.folder ?? '')),
      field('This computer', h('span', st.device_name ?? '')),
      field('Other computers', st.devices.length
        ? h('div', st.devices.map((d) => h('div', `${d.name} · last change ${relative(d.last_seen)}`)))
        : h('span.muted', 'None yet. On another computer, choose this folder in Settings → Sync and type the sync code.')),
      st.last_pass && h('p.small.muted', `Checked ${relative(st.last_pass)}.`)),
    st.offered_accounts.length > 0 && [
      h('h2', 'Accounts on your other computers'),
      h('div.card-list', st.offered_accounts.map((o) =>
        h('div.account-card',
          icon('user'),
          h('div.grow', h('strong', o.config.email), h('div.small.muted', o.config.display_name)),
          h('button.primary', { title: 'Sign in to add this account here, with the same server setup', on: { click: () => signInOffered(o) } }, 'Sign in')))),
    ],
    h('div.row',
      h('button', { title: 'Look for changes from your other computers now', on: { click: () => void now() } }, icon('refresh', 16), 'Sync now'),
      h('button.danger', { title: 'Stop syncing this computer', on: { click: () => void stop() } }, 'Stop syncing')),
  ];
}

function showCode(code: string) {
  modal('Your sync code', h('div.form',
    h('p', 'Type this code on each of your other computers (Settings → Sync → Join). It isn’t shown again, so keep it somewhere safe, like a password manager.'),
    copyBlock(code, 'code.token'),
    h('p.small.muted', 'Anyone with the code and the folder can read your synced settings. Lost it? Stop syncing everywhere and start again for a new one.')));
}
