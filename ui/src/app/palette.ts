import { api } from '../api';
import { debounce, h, icon, patch } from '../lib/dom';
import { fuzzy } from '../lib/fuzzy';
import { openPerson } from '../lib/person';
import { type Command, activeCommands, modal, trigger } from '../lib/ui';
import { type MailSource, type SettingsSection, type View, store } from '../state';
import { openComposer } from '../views/mail/composer';
import { toggleTheme } from './help';
import { UNIFIED, readJson, writeJson } from './location';

/** What the palette needs from the shell. */
export interface PaletteHost {
  go(view: View, source?: MailSource): void;
}

const RECENT_KEY = 'satchel.paletteRecent';
const RECENT_MAX = 8;
const SETTINGS: [SettingsSection, string][] = [['accounts', 'Accounts'], ['agents', 'AI agents'], ['mirror', 'AI folder'], ['privacy', 'Privacy'], ['general', 'General'], ['activity', 'Activity']];

/** Commands that aren't a shortcut or a view's: places to go, and actions
 *  that start in another view. */
function placesAndTools(host: PaletteHost): Command[] {
  const multi = store.state.accounts.length > 1;
  const account = (id: string) => (multi ? store.state.accounts.find((a) => a.id === id)?.email : undefined);
  return [
    ...UNIFIED.map((u) => ({ label: `Go to ${u.label}`, icon: u.icon, run: () => host.go('mail', { kind: 'role', role: u.role, label: u.label }) })),
    ...store.state.mailboxes.filter((m) => m.role === 'custom').map((m) => ({ label: `Go to ${m.path}`, detail: account(m.account_id), icon: m.is_label ? 'label' : 'folder', run: () => host.go('mail', { kind: 'mailbox', mailbox_id: m.id, label: m.path }) })),
    ...SETTINGS.map(([section, label]) => ({ label: `Settings: ${label}`, icon: 'settings', run: () => store.set({ view: 'settings', settingsSection: section }) })),
    // The calendar's and contacts' own shortcuts make these, once the view is up.
    { label: 'New event', icon: 'calendar', run: () => { host.go('calendar'); requestAnimationFrame(() => trigger('n')); } },
    { label: 'New contact', icon: 'user', run: () => { host.go('contacts'); requestAnimationFrame(() => trigger('n')); } },
    { label: 'Sync all accounts now', icon: 'refresh', run: () => void api.syncNow(null) },
    { label: 'Toggle dark mode', icon: 'moon', run: () => toggleTheme() },
  ];
}

/** The command palette (mod+k): every action of the current view, with its
 *  shortcut, places to go, people, and mail search. */
export function palette(host: PaletteHost) {
  const { context, commands } = activeCommands();
  const seen = new Set(commands.map((c) => c.label));
  const all = [...commands, ...placesAndTools(host).filter((c) => !seen.has(c.label))];
  const recent = readJson<string[]>(RECENT_KEY) ?? [];
  const input = h('input.palette-input', { type: 'text', placeholder: 'Type a command, a person, or search mail…', 'aria-label': 'Command' });
  const list = h('div.palette-list', { role: 'listbox' });
  let people: Command[] = [];
  let peopleFor = '';
  let shown: { c: Command; at: number[] }[] = [];
  let idx = 0;

  const run = (c: Command) => {
    writeJson(RECENT_KEY, [c.label, ...recent.filter((l) => l !== c.label)].slice(0, RECENT_MAX));
    m.close();
    c.run();
  };

  const render = () => {
    const q = input.value.trim();
    if (!q) {
      // Recently used first, then everything in its usual order.
      const rank = (c: Command) => { const i = recent.indexOf(c.label); return i < 0 ? RECENT_MAX : i; };
      shown = all.map((c) => ({ c, at: [] as number[] })).sort((a, b) => rank(a.c) - rank(b.c));
    } else {
      const scored = [...all, ...(peopleFor === q ? people : [])].flatMap((c) => {
        const f = fuzzy(q, c.label) ?? (people.includes(c) ? { score: 0, at: [] } : null);
        return f ? [{ c, at: f.at, score: f.score + (recent.includes(c.label) ? 5 : 0) }] : [];
      });
      scored.sort((a, b) => b.score - a.score);
      shown = scored;
      shown.push({ c: { label: `Search mail for “${q}”`, icon: 'search', run: () => host.go('mail', { kind: 'search', query: q, label: `Search: ${q}` }) }, at: [] });
    }
    idx = Math.min(idx, Math.max(shown.length - 1, 0));
    patch(list, shown.map(({ c, at }, i) =>
      h(`button.palette-item${i === idx ? '.active' : ''}`,
        { role: 'option', 'aria-selected': String(i === idx), title: c.hint ? `${c.label} (${c.hint})` : c.label, on: { click: () => run(c), mousemove: () => { if (idx !== i) { idx = i; render(); } } } },
        h('span.palette-icon', c.icon ? icon(c.icon, 16) : null),
        h('span.palette-label', highlight(c.label, at)),
        c.detail && h('span.palette-detail', c.detail),
        c.hint && h('kbd', c.hint))));
  };

  // People come from the core's contact search, so they arrive a beat later.
  const findPeople = debounce(async (q: string) => {
    const found = await api.listContacts(q, 6).catch(() => []);
    if (input.value.trim() !== q) return;
    people = found.filter((p) => p.emails.length).flatMap((p) => {
      const name = p.name || p.emails[0].email;
      const addrs = p.emails.map((e) => ({ name: p.name || null, email: e.email }));
      return [
        { label: `Mail with ${name}`, detail: p.emails[0].email, icon: 'user', run: () => openPerson(addrs, name) },
        { label: `Write to ${name}`, detail: p.emails[0].email, icon: 'compose', run: () => openComposer({ to: [addrs[0]] }) },
      ];
    });
    peopleFor = q;
    render();
  }, 120);

  input.addEventListener('input', () => {
    idx = 0;
    const q = input.value.trim();
    if (q.length >= 2) findPeople(q);
    render();
  });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') idx = Math.min(idx + 1, shown.length - 1);
    else if (e.key === 'ArrowUp') idx = Math.max(idx - 1, 0);
    else if (e.key === 'Enter') {
      const c = shown[idx]?.c;
      if (c) run(c);
      return;
    } else return;
    e.preventDefault();
    render();
    list.querySelector('.active')?.scrollIntoView({ block: 'nearest' });
  });
  const m = modal('Commands', h('div.palette', input, context && h('div.palette-context', `Acting on ${context}`), list));
  m.root.classList.add('palette-modal');
  // Now, not a frame later as dialogs do: people type straight after mod+k.
  input.focus();
  render();
}

/** The label with the matched characters in bold. */
function highlight(label: string, at: number[]) {
  if (!at.length) return label;
  const out: (string | HTMLElement)[] = [];
  const hit = new Set(at);
  let run = '';
  let bold = false;
  for (let i = 0; i <= label.length; i++) {
    const b = hit.has(i);
    if (i === label.length || b !== bold) {
      if (run) out.push(bold ? h('b', run) : run);
      run = '';
      bold = b;
    }
    run += label[i] ?? '';
  }
  return out;
}
