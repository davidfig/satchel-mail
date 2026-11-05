// The person view's header: who they are, filters, shared meetings.

import { api } from '../../api';
import { h, icon } from '../../lib/dom';
import { displayName } from '../../lib/format';
import { avatar, isFavorite, setFavorite } from '../../lib/person';
import { errorToast } from '../../lib/ui';
import { store } from '../../state';
import type { MailSource, PersonFilter } from '../../state';
import { openComposer } from './composer';
import type { MailView } from './index';

export function loadPerson(view: MailView, emails: string[]) {
  const key = emails.join(',');
  if (view.person?.key === key) return;
  view.person = { key, summary: null };
  api.getPerson(emails).then((summary) => {
    if (view.person?.key !== key) return;
    const all = summary.addresses.map((a) => a.email);
    const s = store.state.source;
    if (s.kind === 'person' && all.join(',') !== key) {
      // They're a contact with more addresses: list mail to and from all of them.
      view.person = { key: all.join(','), summary };
      store.set({ source: { ...s, emails: all } });
      return;
    }
    view.person.summary = summary;
    view.renderHeader();
  }, errorToast);
}

export function personCard(view: MailView, s: Extract<MailSource, { kind: 'person' }>) {
  const p = view.person?.summary;
  const main = p?.addresses[0] ?? { email: s.emails[0], name: s.label };
  const setFilter = (filter: PersonFilter) => store.set({ source: { ...s, filter } });
  const filters: [PersonFilter, string, string][] = [
    ['all', 'All', 'Conversations they are in'],
    ['from', 'From them', 'Conversations with mail they sent'],
    ['files', 'Attachments', 'Conversations with attachments'],
  ];
  const addresses = p?.addresses ?? s.emails.map((email) => ({ email }));
  const fav = isFavorite(addresses);
  const since = p?.first_date && new Date(p.first_date).toLocaleDateString([], { year: 'numeric', month: 'short' });
  return h('div.person-card',
    h('div.person-id',
      avatar(main, true),
      h('div.grow',
        h('div.person-emails', addresses.map((a) => a.email).join(', ')),
        h('div.muted.small', p
          ? [`${count(p.threads, 'conversation')} · ${p.received} received · ${p.sent} sent`, since ? ` · since ${since}` : ''].join('')
          : 'Loading…')),
      h('button.icon-btn.fav-toggle', {
        title: fav ? 'Remove from favorites' : 'Add to favorites: highlight their mail',
        'aria-label': fav ? 'Remove from favorites' : 'Add to favorites',
        'aria-pressed': String(fav),
        on: { click: () => void setFavorite(addresses, !fav) },
      }, icon('heart')),
      p?.contact_id && h('button.icon-btn', { title: 'Open contact', on: { click: () => store.set({ view: 'contacts', selectedContact: p.contact_id }) } }, icon('user')),
      h('button.icon-btn', { title: `Write to ${displayName(main)}`, 'aria-label': 'Write to them', on: { click: () => openComposer({ to: [main] }) } }, icon('compose'))),
    h('div.segmented', { role: 'group', 'aria-label': 'Show' },
      filters.map(([f, label, tip]) => h(`button${s.filter === f ? '.active' : ''}`, { title: tip, 'aria-pressed': String(s.filter === f), on: { click: () => setFilter(f) } }, label))),
    p && p.events.length > 0 &&
      h('details.person-events', { open: view.eventsOpen, on: { toggle: (e: Event) => { view.eventsOpen = (e.currentTarget as HTMLDetailsElement).open; } } },
        h('summary', `${count(p.events.length, 'meeting')}${p.events.length >= 20 ? '+' : ''}`),
        h('ul', p.events.map((e) => h('li',
          h('span.grow', e.title || '(no title)'),
          h('time.muted.small', { datetime: e.start }, (e.recurring ? 'Repeats from ' : '') + new Date(e.start).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' })))))),
  );
}

/** The search behind a person view: every address, any role. */
export function personQuery(emails: string[], filter: PersonFilter): string {
  const list = emails.join(',');
  if (filter === 'from') return `from:${list}`;
  if (filter === 'files') return `with:${list} has:attachment`;
  return `with:${list}`;
}

export const count = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
