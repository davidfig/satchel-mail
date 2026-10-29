// Smart folders: the dialog that makes or edits one, and deleting one.

import { api } from '../api';
import { store } from '../state';
import type { SmartBadge, SmartFolder } from '../types';
import { h } from './dom';
import { AGES, type QueryFields, TEMPLATES, formatFields, parseFields } from './smart-query';
import { confirmDialog, errorToast, modal, toast } from './ui';

/** New smart folder (optionally from a search) or edit one. Resolves to the saved folder. */
export function smartFolderDialog(opts: { folder?: SmartFolder; query?: string } = {}): Promise<SmartFolder | null> {
  const existing = opts.folder;
  const emails = store.state.accounts.map((a) => a.email);
  const name = h('input', { type: 'text', value: existing?.name ?? '', placeholder: 'Smart folder name', 'aria-label': 'Name', autocomplete: 'off' });
  const query = h('input.mono', { type: 'text', value: existing?.query ?? opts.query ?? '', placeholder: 'from:bob is:unread', 'aria-label': 'Search', autocomplete: 'off', spellcheck: 'false' });
  const text = (placeholder: string, label: string) => h('input', { type: 'text', placeholder, 'aria-label': label, autocomplete: 'off' });
  const check = () => h('input', { type: 'checkbox' });
  const f = {
    from: text('Name or address, comma-separated', 'From'),
    to: text('Name or address, comma-separated', 'To'),
    subject: text('Words in the subject', 'Subject'),
    words: text('Anywhere in the message', 'Has the words'),
    unread: check(),
    flagged: check(),
    attachment: check(),
    age: h('select', { 'aria-label': 'Received' }, AGES.map(([v, l]) => h('option', { value: v }, l))),
    anywhere: check(),
    accounts: emails.map((email) => ({ email, box: check() })),
  };
  const badge = h('select', { 'aria-label': 'Badge' },
    h('option', { value: 'unread' }, 'Unread conversations'),
    h('option', { value: 'total' }, 'All conversations'),
    h('option', { value: 'none' }, 'No count'));
  badge.value = existing?.badge ?? 'unread';
  // Terms only the query box can say, carried through edits to the fields.
  let other: string[] = [];

  const fill = (v: QueryFields) => {
    f.from.value = v.from;
    f.to.value = v.to;
    f.subject.value = v.subject;
    f.words.value = v.words;
    f.unread.checked = v.unread;
    f.flagged.checked = v.flagged;
    f.attachment.checked = v.attachment;
    f.age.value = v.age;
    f.anywhere.checked = v.anywhere;
    const chosen = new Set(v.accounts.map((a) => a.toLowerCase()));
    for (const a of f.accounts) a.box.checked = chosen.has(a.email.toLowerCase());
    other = v.other;
  };
  const read = (): QueryFields => ({
    from: f.from.value,
    to: f.to.value,
    subject: f.subject.value,
    words: f.words.value,
    unread: f.unread.checked,
    flagged: f.flagged.checked,
    attachment: f.attachment.checked,
    age: f.age.value,
    anywhere: f.anywhere.checked,
    // Every account ticked is the same as none: search them all.
    accounts: f.accounts.every((a) => a.box.checked) ? [] : f.accounts.filter((a) => a.box.checked).map((a) => a.email),
    other,
  });
  fill(parseFields(query.value, emails));
  query.addEventListener('input', () => fill(parseFields(query.value, emails)));
  const fromFields = () => { query.value = formatFields(read()); };
  for (const el of [f.from, f.to, f.subject, f.words]) el.addEventListener('input', fromFields);
  for (const el of [f.unread, f.flagged, f.attachment, f.age, f.anywhere, ...f.accounts.map((a) => a.box)]) el.addEventListener('change', fromFields);

  return new Promise((resolve) => {
    let done = false;
    const finish = (v: SmartFolder | null) => {
      if (done) return;
      done = true;
      m.close();
      resolve(v);
    };
    const save = async () => {
      if (!name.value.trim()) return name.focus();
      if (!query.value.trim()) return query.focus();
      try {
        const folder: SmartFolder = { id: existing?.id ?? '', name: name.value, query: query.value, badge: badge.value as SmartBadge, count: 0 };
        finish(await api.saveSmartFolder(folder));
      } catch (e) {
        errorToast(e);
      }
    };
    for (const el of [name, query, f.from, f.to, f.subject, f.words]) {
      el.addEventListener('keydown', (e) => {
        if ((e as KeyboardEvent).key === 'Enter') void save();
      });
    }
    const field = (label: string, el: Node) => h('label.form-field', h('span.form-label', label), el);
    const tick = (label: string, el: HTMLInputElement) => h('label.row.inline', el, h('span', label));
    const templates = !existing && !opts.query &&
      h('div.smart-templates',
        h('span.form-label', 'Start from'),
        TEMPLATES.map((t) => h('button.chip', {
          title: `Search for ${t.query}`,
          on: { click: () => { if (!name.value.trim()) name.value = t.name; query.value = t.query; fill(parseFields(t.query, emails)); } },
        }, t.name)));
    const body = h('div.form.smart-form',
      field('Name', name),
      templates,
      h('div.row', field('From', f.from), field('To', f.to)),
      h('div.row', field('Subject', f.subject), field('Has the words', f.words)),
      h('div.row', field('Received', f.age), field('Badge shows', badge)),
      h('div.smart-checks', tick('Unread', f.unread), tick('Flagged', f.flagged), tick('Has attachments', f.attachment), tick('Include Trash and Spam', f.anywhere)),
      emails.length > 1 && h('div.form-field', h('span.form-label', 'Accounts'),
        h('div.smart-checks', f.accounts.map((a) => tick(a.email, a.box)))),
      field('Search', query),
      h('p.muted.small', 'Searches every folder of every account. The fields above write this search; you can also type one, e.g. -from:noreply or older_than:1y.'));
    const footer = h('div.row.end',
      h('button', { title: 'Close without saving (Esc)', on: { click: () => finish(null) } }, 'Cancel'),
      h('button.primary', { title: 'Save the smart folder (Enter)', on: { click: () => void save() } }, existing ? 'Save' : 'Create smart folder'));
    const m = modal(existing ? `Edit “${existing.name}”` : 'New smart folder', body, { footer, onClose: () => finish(null) });
    requestAnimationFrame(() => { name.focus(); name.select(); });
  });
}

/** Ask, then delete a smart folder. Its mail stays where it is. */
export async function deleteSmartFolder(f: SmartFolder): Promise<boolean> {
  if (!(await confirmDialog(`Delete “${f.name}”?`, 'Only the smart folder goes. Your mail stays where it is.', 'Delete smart folder', true))) return false;
  try {
    await api.deleteSmartFolder(f.id);
    toast(`Deleted “${f.name}”.`);
    return true;
  } catch (e) {
    errorToast(e);
    return false;
  }
}

/** Move a smart folder one place up (-1) or down (1). */
export function moveSmartFolder(f: SmartFolder, delta: -1 | 1) {
  const ids = store.state.smartFolders.map((x) => x.id);
  const i = ids.indexOf(f.id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= ids.length) return;
  [ids[i], ids[j]] = [ids[j], ids[i]];
  api.reorderSmartFolders(ids).catch(errorToast);
}
