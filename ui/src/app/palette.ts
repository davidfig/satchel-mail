import { api } from '../api';
import { h, replace } from '../lib/dom';
import { modal } from '../lib/ui';
import { type MailSource, type View, store } from '../state';
import { openComposer } from '../views/mail/composer';
import { showShortcuts, toggleTheme } from './help';
import { UNIFIED } from './location';

/** What the palette needs from the shell. */
export interface PaletteHost {
  go(view: View, source?: MailSource): void;
  toggleSidebar(): void;
}

/** The command palette (mod+k): jump anywhere or search mail. */
export function palette(host: PaletteHost) {
  const actions: { label: string; run: () => void }[] = [
    { label: 'Compose new message', run: () => openComposer({}) },
    ...UNIFIED.map((u) => ({ label: `Go to ${u.label}`, run: () => host.go('mail', { kind: 'role', role: u.role, label: u.label }) })),
    ...store.state.mailboxes.filter((m) => m.role === 'custom').map((m) => ({ label: `Go to ${m.path}`, run: () => host.go('mail', { kind: 'mailbox', mailbox_id: m.id, label: m.path }) })),
    { label: 'Open Calendar', run: () => host.go('calendar') },
    { label: 'Open Contacts', run: () => host.go('contacts') },
    { label: 'New contact', run: () => { host.go('contacts'); requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('.contacts-view .list-title .icon-btn')?.click()); } },
    { label: 'Open Review queue', run: () => host.go('review') },
    { label: 'Open Settings', run: () => host.go('settings') },
    { label: 'Sync all accounts now', run: () => void api.syncNow(null) },
    { label: 'Toggle dark mode', run: () => toggleTheme() },
    { label: 'Collapse or expand sidebar', run: () => host.toggleSidebar() },
    { label: 'Keyboard shortcuts', run: () => showShortcuts() },
  ];
  const input = h('input.palette-input', { type: 'text', placeholder: 'Type a command or search mail…', 'aria-label': 'Command' });
  const list = h('div.palette-list', { role: 'listbox' });
  let idx = 0;
  let filtered = actions;
  const render = () => {
    const q = input.value.toLowerCase().trim();
    filtered = actions.filter((a) => a.label.toLowerCase().includes(q));
    if (q) filtered.push({ label: `Search mail for “${input.value.trim()}”`, run: () => host.go('mail', { kind: 'search', query: input.value.trim(), label: `Search: ${input.value.trim()}` }) });
    idx = Math.min(idx, Math.max(filtered.length - 1, 0));
    replace(list, filtered.map((a, i) => h(`button.palette-item${i === idx ? '.active' : ''}`, { role: 'option', on: { click: () => { m.close(); a.run(); } } }, a.label)));
  };
  input.addEventListener('input', () => { idx = 0; render(); });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') idx = Math.min(idx + 1, filtered.length - 1);
    else if (e.key === 'ArrowUp') idx = Math.max(idx - 1, 0);
    else if (e.key === 'Enter') {
      m.close();
      filtered[idx]?.run();
      return;
    } else return;
    e.preventDefault();
    render();
  });
  const m = modal('Commands', h('div.palette', input, list));
  m.root.classList.add('palette-modal');
  render();
}
