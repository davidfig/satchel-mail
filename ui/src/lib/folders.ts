// Folders: the tree shown in the sidebar, and the dialogs to create, rename,
// move and delete them, and emptying Spam and Trash.

import { api } from '../api';
import { type MailSource, store } from '../state';
import type { Mailbox } from '../types';
import { h } from './dom';
import { confirmDialog, errorToast, modal, toast } from './ui';

/** The user's own folders, in tree order, each with how deep it sits under
 *  the others shown (a folder inside Inbox on IMAP still starts at 0). */
export function folderTree(boxes: Mailbox[]): { box: Mailbox; depth: number }[] {
  const own = boxes.filter((m) => m.role === 'custom');
  const paths = new Set(own.map((m) => m.path));
  const depth = (path: string) => {
    let d = 0;
    for (let i = path.indexOf('/'); i >= 0; i = path.indexOf('/', i + 1)) if (paths.has(path.slice(0, i))) d++;
    return d;
  };
  // Sort so a parent comes right before its children ("A", "A/B", "A B").
  const key = (p: string) => p.toLowerCase().split('/').join('\u0000');
  return own.sort((a, b) => (key(a.path) < key(b.path) ? -1 : 1)).map((box) => ({ box, depth: depth(box.path) }));
}

/** New folder (no `folder`) or rename/move one. Resolves to the saved folder. */
export function folderDialog(accountId: string, opts: { folder?: Mailbox; parentId?: string | null } = {}): Promise<Mailbox | null> {
  const f = opts.folder;
  const parentOf = (m: Mailbox) => store.state.mailboxes.find((p) => p.account_id === m.account_id && p.role === 'custom' && m.path.startsWith(`${p.path}/`) && !m.path.slice(p.path.length + 1).includes('/'));
  const inside = (m: Mailbox) => !!f && (m.id === f.id || m.path.startsWith(`${f.path}/`));
  const choices = folderTree(store.state.mailboxes.filter((m) => m.account_id === accountId)).filter(({ box }) => !inside(box));
  const name = h('input', { type: 'text', value: f?.name ?? '', placeholder: 'Folder name', 'aria-label': 'Folder name', autocomplete: 'off' });
  const parent = h('select', { 'aria-label': 'Inside' },
    h('option', { value: '' }, 'Top level'),
    choices.map(({ box, depth }) => h('option', { value: box.id }, `${' '.repeat(depth)}${box.name}`)));
  parent.value = (f ? parentOf(f)?.id : opts.parentId) ?? '';
  return new Promise((resolve) => {
    let done = false;
    const finish = (v: Mailbox | null) => {
      if (done) return;
      done = true;
      m.close();
      resolve(v);
    };
    const save = async () => {
      const n = name.value.trim();
      if (!n) return name.focus();
      const parentId = parent.value || null;
      try {
        finish(f ? await api.updateMailbox(f.id, n, parentId) : await api.createMailbox(accountId, n, parentId));
      } catch (e) {
        errorToast(e);
        name.focus();
      }
    };
    name.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void save();
    });
    const account = store.state.accounts.length > 1 ? store.state.accounts.find((a) => a.id === accountId)?.email : null;
    const body = h('div.form',
      account && h('p.muted.small', `In ${account}`),
      h('label.form-field', h('span.form-label', 'Name'), name),
      h('label.form-field', h('span.form-label', 'Inside'), parent));
    const footer = h('div.row.end',
      h('button', { on: { click: () => finish(null) } }, 'Cancel'),
      h('button.primary', { on: { click: () => void save() } }, f ? 'Save' : 'Create folder'));
    const m = modal(f ? `Rename “${f.name}”` : 'New folder', body, { footer, onClose: () => finish(null) });
    requestAnimationFrame(() => { name.focus(); name.select(); });
  });
}

/** Ask, then delete a folder; its mail goes to Trash (a label just goes away). */
export async function deleteFolder(f: Mailbox): Promise<boolean> {
  if (store.state.mailboxes.some((m) => m.account_id === f.account_id && m.path.startsWith(`${f.path}/`))) {
    toast(`“${f.name}” has folders inside it. Move or delete those first.`, { kind: 'error' });
    return false;
  }
  const n = f.total;
  const what = f.is_label
    ? `The label is removed from ${n === 1 ? 'its message' : `its ${n} messages`}; the messages stay.`
    : n ? `Its ${n === 1 ? 'message moves' : `${n} messages move`} to Trash.` : 'It’s empty.';
  if (!(await confirmDialog(`Delete “${f.name}”?`, what, 'Delete folder', true))) return false;
  try {
    const r = await api.deleteMailbox(f.id);
    toast(`Deleted “${f.name}”.${r.moved_to_trash ? ` ${r.moved_to_trash} message${r.moved_to_trash === 1 ? '' : 's'} moved to Trash.` : ''}`);
    return true;
  } catch (e) {
    errorToast(e);
    return false;
  }
}

/** Spam or Trash, when `source` is one of them (all accounts' or one). */
export function emptiable(source: MailSource): { label: string; count: number } | null {
  const role = source.kind === 'role' ? source.role : source.kind === 'mailbox' ? store.mailbox(source.mailbox_id)?.role : null;
  if (role !== 'junk' && role !== 'trash') return null;
  const boxes = source.kind === 'mailbox' ? store.state.mailboxes.filter((m) => m.id === source.mailbox_id) : store.state.mailboxes.filter((m) => m.role === role);
  return { label: role === 'junk' ? 'Spam' : 'Trash', count: boxes.reduce((n, m) => n + m.total, 0) };
}

/** Ask, then permanently delete everything in Spam or Trash. */
export async function emptyFolder(source: MailSource): Promise<void> {
  const e = emptiable(source);
  if (!e || (source.kind !== 'role' && source.kind !== 'mailbox')) return;
  const what = `${e.count === 1 ? 'The message' : `All ${e.count} messages`} in ${e.label} will be permanently deleted.`;
  if (!(await confirmDialog(`Empty ${e.label}?`, what, `Empty ${e.label}`, true))) return;
  try {
    const r = await api.emptyMailbox(source.kind === 'mailbox' ? source.mailbox_id : null, source.kind === 'role' ? source.role : null);
    toast(`Deleted ${r.deleted} message${r.deleted === 1 ? '' : 's'}.`);
  } catch (err) {
    errorToast(err);
  }
}
