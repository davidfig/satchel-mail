// Contacts: everyone in your address books plus the people you write to,
// with search, editing, merging and deleting.

import { api } from '../api';
import { debounce, h, icon, replace } from '../lib/dom';
import { relative } from '../lib/format';
import { avatar, openPerson } from '../lib/person';
import { bind, confirmDialog, errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { AddressBook, Contact, ContactEmail, ContactPhone } from '../types';
import { openComposer } from './mail/composer';

const EMAIL_LABELS = ['', 'home', 'work', 'other'];
const PHONE_LABELS = ['', 'cell', 'home', 'work', 'other'];

export function contactName(c: Contact): string {
  return c.name || c.emails[0]?.email || '(no name)';
}

function contactAvatar(c: Contact, big = false) {
  return avatar({ name: c.name || null, email: c.emails[0]?.email ?? c.name }, big);
}

function blank(): Contact {
  return { id: null, name: '', emails: [{ email: '', label: null }], phones: [], organization: '', title: '', birthday: null, notes: '' };
}

export class ContactsView {
  root = h('div.contacts-view');
  private list: Contact[] = [];
  private books: AddressBook[] = [];
  private selected: string | null = null;
  /** The contact being edited (a copy), or null when reading. */
  private editing: Contact | null = null;
  private search = h('input.search', { type: 'search', placeholder: 'Search contacts', 'aria-label': 'Search contacts' });
  private listEl = h('div.list-scroll.contact-list', { role: 'listbox', 'aria-label': 'Contacts' });
  private detail = h('section.reader.contact-detail');
  private countEl = h('span.muted.small');

  constructor() {
    this.search.addEventListener('input', debounce(() => void this.load(), 150));
    this.search.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && this.list[0]?.id) this.select(this.list[0].id);
      if (e.key === 'Escape') this.search.blur();
    });
    replace(this.root,
      h('div.list-pane',
        h('header.list-head',
          h('div.list-title', h('h1', 'Contacts'), this.countEl, h('span.spacer'),
            h('button.icon-btn', { title: 'New contact (n)', on: { click: () => this.startNew() } }, icon('plus', 18))),
          this.search),
        this.listEl),
      this.detail);
    store.onEvent((e) => {
      if ((e.event === 'contacts_changed' || e.event === 'data_changed') && store.state.view === 'contacts' && !this.editing) void this.load();
    });
    const s = 'contacts';
    bind('/', 'Search contacts', () => this.search.focus(), s);
    bind('n', 'New contact', () => this.startNew(), s);
    bind('e', 'Edit contact', () => this.startEdit(), s);
    bind('j', 'Next contact', () => this.move(1), s);
    bind('k', 'Previous contact', () => this.move(-1), s);
    bind('ArrowDown', 'Next contact', () => this.move(1), s);
    bind('ArrowUp', 'Previous contact', () => this.move(-1), s);
  }

  show() {
    void this.load();
  }

  private async load() {
    try {
      [this.list, this.books] = await Promise.all([api.listContacts(this.search.value.trim() || null), api.listAddressBooks()]);
    } catch (e) {
      errorToast(e);
      return;
    }
    // Another view asked for this contact (the person view's "Open contact").
    const wanted = store.state.selectedContact;
    if (wanted) {
      store.set({ selectedContact: null });
      if (this.list.some((c) => c.id === wanted)) {
        this.selected = wanted;
        this.editing = null;
      }
    }
    if (!this.selected || !this.list.some((c) => c.id === this.selected)) this.selected = this.list[0]?.id ?? null;
    this.renderList();
    if (!this.editing) this.renderDetail();
  }

  private current(): Contact | undefined {
    return this.list.find((c) => c.id === this.selected);
  }

  private move(d: number) {
    if (this.editing || !this.list.length) return;
    const i = this.list.findIndex((c) => c.id === this.selected);
    const next = this.list[Math.max(0, Math.min(this.list.length - 1, i + d))];
    if (next?.id) this.select(next.id);
  }

  private select(id: string) {
    this.selected = id;
    this.editing = null;
    this.renderList();
    this.renderDetail();
    this.listEl.querySelector('.active')?.scrollIntoView({ block: 'nearest' });
  }

  private renderList() {
    this.countEl.textContent = this.list.length ? String(this.list.length) : '';
    if (!this.list.length) {
      replace(this.listEl, h('div.empty-list', this.search.value ? 'No contacts match.' : 'No contacts yet. People you write to appear here automatically.'));
      return;
    }
    replace(this.listEl, this.list.map((c) =>
      h(`button.contact-row${c.id === this.selected ? '.active' : ''}`, { role: 'option', 'aria-selected': String(c.id === this.selected), on: { click: () => c.id && this.select(c.id) } },
        contactAvatar(c),
        h('span.contact-row-main',
          h('span.who', contactName(c)),
          h('span.muted.small', c.name ? (c.emails[0]?.email ?? c.organization) : c.organization)),
        c.origin === 'auto' && h('span.chip', { title: 'Added because you wrote to this person' }, 'from mail'))));
  }

  private renderDetail() {
    const c = this.current();
    if (!c) {
      replace(this.detail, h('div.reader-empty', icon('user', 36), h('p', 'No contact selected')));
      return;
    }
    const accountEmail = (id: string) => store.state.accounts.find((a) => a.id === id)?.email ?? '';
    const where = c.sources?.length
      ? c.sources.map((s) => h('span.chip', { title: `Saved in this address book of ${accountEmail(s.account_id)}` }, `${s.name} · ${accountEmail(s.account_id)}`))
      : [h('span.chip', c.origin === 'auto' ? 'Added from your mail' : 'Only in Satchel')];
    const first = c.emails[0]?.email;
    replace(this.detail,
      h('header.reader-head',
        h('div.contact-head', contactAvatar(c, true),
          h('div.reader-title',
            h('h2.reader-subject', contactName(c)),
            (c.title || c.organization) && h('div.muted', [c.title, c.organization].filter(Boolean).join(', ')))),
        h('div.reader-actions',
          first && h('button.icon-btn', { title: `Write to ${contactName(c)}`, on: { click: () => openComposer({ to: [{ name: c.name || null, email: first }] }) } }, icon('compose')),
          first && h('button.icon-btn', { title: 'All mail with this person', on: { click: () => openPerson(c.emails.map((e) => ({ name: c.name || null, email: e.email })), contactName(c)) } }, icon('mail')),
          h('button.icon-btn', { title: 'Edit (e)', on: { click: () => this.startEdit() } }, icon('draft')),
          h('button.icon-btn', { title: 'Merge with other contacts', on: { click: () => this.merge(c) } }, icon('user')),
          h('button.icon-btn', { title: 'Delete contact', on: { click: () => void this.remove(c) } }, icon('trash')))),
      h('div.contact-body',
        section('Email', c.emails.map((e) => h('div.contact-line',
          h('button.link', { title: `Write to ${e.email}`, on: { click: () => openComposer({ to: [{ name: c.name || null, email: e.email }] }) } }, e.email),
          e.label && h('span.muted.small', e.label)))),
        section('Phone', c.phones.map((p) => h('div.contact-line', h('span', p.number), p.label && h('span.muted.small', p.label)))),
        c.birthday && section('Birthday', [h('div', formatBirthday(c.birthday))]),
        c.notes && section('Notes', [h('div.contact-notes', c.notes)]),
        section('Saved in', [h('div.labels', where)]),
        h('p.muted.small', c.message_count ? `${c.message_count} message${c.message_count === 1 ? '' : 's'} exchanged · last ${relative(c.last_contacted ?? null)}` : 'No messages exchanged yet'),
        !!c.message_count && h('button.link', { title: 'All mail with this person', on: { click: () => openPerson(c.emails.map((e) => ({ name: c.name || null, email: e.email })), contactName(c)) } }, 'Show all mail')));
  }

  private startNew() {
    this.editing = blank();
    this.renderEditor();
  }

  private startEdit() {
    const c = this.current();
    if (!c) return;
    this.editing = structuredClone(c);
    if (!this.editing.emails.length) this.editing.emails.push({ email: '', label: null });
    this.renderEditor();
  }

  private renderEditor() {
    const c = this.editing!;
    const isNew = !c.id;
    const input = (value: string, set: (v: string) => void, attrs: Record<string, unknown> = {}) => {
      const el = h('input', { type: 'text', value, ...attrs });
      el.addEventListener('input', () => set(el.value));
      return el;
    };
    const labelSelect = (options: string[], value: string | null | undefined, set: (v: string | null) => void) => {
      const el = h('select', { 'aria-label': 'Label' }, options.map((o) => h('option', { value: o, selected: (value ?? '') === o }, o || 'no label')));
      el.addEventListener('change', () => set(el.value || null));
      return el;
    };
    const rows = <T extends ContactEmail | ContactPhone>(items: T[], key: 'email' | 'number', labels: string[], placeholder: string, add: () => T): HTMLElement => {
      const wrap = h('div.contact-rows');
      const draw = () =>
        replace(wrap,
          items.map((it, i) => h('div.row',
            input((it as unknown as Record<string, string>)[key], (v) => ((it as unknown as Record<string, string>)[key] = v), { placeholder, 'aria-label': placeholder }),
            labelSelect(labels, it.label, (v) => (it.label = v)),
            h('button.icon-btn', { title: `Remove this ${key === 'email' ? 'address' : 'number'}`, on: { click: () => { items.splice(i, 1); draw(); } } }, icon('close', 16)))),
          h('button.link', { title: `Add ${key === 'email' ? 'an address' : 'a number'}`, on: { click: () => { items.push(add()); draw(); } } }, icon('plus', 14), key === 'email' ? 'Add email' : 'Add phone'));
      draw();
      return wrap;
    };
    const inBooks = new Set((c.sources ?? []).map((s) => s.address_book_id));
    const writable = this.books.filter((b) => !b.read_only && !inBooks.has(b.id));
    const accountEmail = (id: string) => store.state.accounts.find((a) => a.id === id)?.email ?? '';
    const book = h('select', { 'aria-label': 'Save to' },
      h('option', { value: '' }, c.sources?.length ? 'No other address book' : 'Only in Satchel'),
      writable.map((b) => h('option', { value: b.id }, `${b.name} · ${accountEmail(b.account_id)}`)));
    const notes = h('textarea', { rows: 4, 'aria-label': 'Notes' });
    notes.value = c.notes;
    notes.addEventListener('input', () => (c.notes = notes.value));
    const status = h('p.small.error');
    const save = async () => {
      try {
        const saved = await api.saveContact(c, book.value || null);
        this.editing = null;
        this.selected = saved.id ?? null;
        toast(isNew ? 'Contact added' : 'Contact saved', { kind: 'success' });
        await this.load();
      } catch (e) {
        status.textContent = e instanceof Error ? e.message : String(e);
      }
    };
    const field = (label: string, el: Node, hint?: string) => h('label.form-field', h('span.form-label', label), el, hint && h('span.form-hint', hint));
    const form = h('div.form.contact-form',
      field('Name', input(c.name, (v) => (c.name = v), { placeholder: 'Full name' })),
      h('div.form-field', h('span.form-label', 'Email'), rows(c.emails, 'email', EMAIL_LABELS, 'name@example.com', () => ({ email: '', label: null }))),
      h('div.form-field', h('span.form-label', 'Phone'), rows(c.phones, 'number', PHONE_LABELS, '+1 555 0100', () => ({ number: '', label: null }))),
      h('div.row', field('Title', input(c.title, (v) => (c.title = v))), field('Organization', input(c.organization, (v) => (c.organization = v)))),
      field('Birthday', input(c.birthday ?? '', (v) => (c.birthday = v.trim() || null), { placeholder: 'YYYY-MM-DD' }), 'Use --MM-DD to leave out the year.'),
      field('Notes', notes),
      (writable.length > 0 || !c.sources?.length) && field(c.sources?.length ? 'Also save to' : 'Save to', book, writable.length ? 'Saving to an account’s address book syncs this contact to your other devices.' : undefined),
      status);
    form.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void save();
      if (e.key === 'Escape') this.cancelEdit();
    });
    replace(this.detail,
      h('header.reader-head',
        h('div.reader-title', h('h2.reader-subject', isNew ? 'New contact' : `Edit ${contactName(c)}`)),
        h('div.row',
          h('button', { title: 'Discard changes (Esc)', on: { click: () => this.cancelEdit() } }, 'Cancel'),
          h('button.primary', { title: `Save (${navigator.platform.includes('Mac') ? '⌘' : 'Ctrl+'}Enter)`, on: { click: () => void save() } }, 'Save'))),
      h('div.contact-body', form));
    (form.querySelector('input') as HTMLInputElement | null)?.focus();
  }

  private cancelEdit() {
    this.editing = null;
    this.renderDetail();
  }

  private async remove(c: Contact) {
    const where = c.sources?.length ? ` It is also deleted from ${c.sources.map((s) => s.name).join(', ')}.` : '';
    if (!(await confirmDialog('Delete contact?', `Delete ${contactName(c)}?${where} Satchel won’t add them back automatically.`, 'Delete', true))) return;
    try {
      await api.deleteContact(c.id!);
      toast('Contact deleted');
      this.selected = null;
      await this.load();
    } catch (e) {
      errorToast(e);
    }
  }

  private merge(primary: Contact) {
    const chosen = new Set<string>();
    const q = h('input.search', { type: 'search', placeholder: 'Find the same person under another name or address', 'aria-label': 'Search contacts' });
    const listEl = h('div.merge-list');
    const go = h('button.primary', { disabled: true, title: 'Combine the selected contacts into this one' }, 'Merge');
    const draw = () => {
      const term = q.value.trim().toLowerCase();
      const words = primary.name.toLowerCase().split(/\s+/).filter((w) => w.length > 2);
      const candidates = this.list
        .filter((c) => c.id !== primary.id)
        .filter((c) => (term ? `${c.name} ${c.emails.map((e) => e.email).join(' ')} ${c.organization}`.toLowerCase().includes(term) : words.some((w) => c.name.toLowerCase().includes(w))))
        .slice(0, 50);
      replace(listEl, candidates.length
        ? candidates.map((c) => {
            const box = h('input', { type: 'checkbox', checked: chosen.has(c.id!) });
            box.addEventListener('change', () => {
              if (box.checked) chosen.add(c.id!);
              else chosen.delete(c.id!);
              go.disabled = chosen.size === 0;
            });
            return h('label.row.inline.merge-item', box, contactAvatar(c), h('span', h('strong', contactName(c)), ' ', h('span.muted.small', c.emails.map((e) => e.email).join(', '))));
          })
        : h('p.muted.small', term ? 'No other contacts match.' : 'Search for the contacts to merge.'));
    };
    q.addEventListener('input', debounce(draw, 100));
    go.addEventListener('click', async () => {
      try {
        const merged = await api.mergeContacts([primary.id!, ...chosen]);
        m.close();
        this.selected = merged.id ?? null;
        toast(`Merged into ${contactName(merged)}`, { kind: 'success' });
        await this.load();
      } catch (e) {
        errorToast(e);
      }
    });
    const m = modal(`Merge into ${contactName(primary)}`, h('div.form',
      h('p.muted.small', 'The selected contacts’ addresses, numbers and notes move into this one, and they’re removed. Copies in your accounts’ address books are combined too.'),
      q, listEl), { footer: h('div.row.end', h('button', { on: { click: () => m.close() } }, 'Cancel'), go) });
    draw();
  }
}

function section(title: string, children: Node[]): HTMLElement | null {
  if (!children.length) return null;
  return h('div.contact-section', h('h3', title), children);
}

function formatBirthday(b: string): string {
  const m = b.match(/^(\d{4})?-?-?(\d{2})-(\d{2})$/);
  if (!m) return b;
  const d = new Date(Number(m[1] ?? 2000), Number(m[2]) - 1, Number(m[3]));
  return d.toLocaleDateString([], m[1] ? { year: 'numeric', month: 'long', day: 'numeric' } : { month: 'long', day: 'numeric' });
}
