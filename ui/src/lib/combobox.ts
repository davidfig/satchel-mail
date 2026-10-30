// A text input with suggestions under it: typing asks `source` for matches,
// arrows move, Enter or a click picks one. The list is a fixed-position
// `.menu` on <body>, so a scrolling dialog doesn't clip it. `source` may be
// async (a lookup); answers to older queries are dropped. With `freeText`,
// nothing is highlighted until an arrow key, so Enter keeps what was typed.
// `twoLine` puts each detail under its label (long addresses); `footer`
// adds a muted line under the list (a source's credit).

import { h, icon as iconEl, listen, patch } from './dom';

export interface Suggestion {
  label: string;
  /** Muted text after (or under) the label. */
  detail?: string;
  /** An icon name from dom.ts, before the label. */
  icon?: string;
  value: string;
}

export interface ComboOptions {
  freeText?: boolean;
  twoLine?: boolean;
  footer?: (items: Suggestion[]) => string | null;
}

export function combobox(input: HTMLInputElement, source: (query: string) => Suggestion[] | Promise<Suggestion[]>, onPick: (s: Suggestion) => void, opts: ComboOptions = {}) {
  const list = h(`div.menu.suggest${opts.twoLine ? '.two-line' : ''}`, { role: 'listbox', on: { mousedown: (e: MouseEvent) => e.preventDefault() } });
  list.id = `combo-${Math.random().toString(36).slice(2)}`;
  input.setAttribute('role', 'combobox');
  input.setAttribute('aria-controls', list.id);
  input.setAttribute('aria-expanded', 'false');
  input.autocomplete = 'off';
  let items: Suggestion[] = [];
  const first = opts.freeText ? -1 : 0;
  let active = first;
  let asked = 0;

  const place = () => {
    const r = input.getBoundingClientRect();
    const w = Math.max(r.width, 240);
    list.style.width = `${w}px`;
    list.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
    const below = innerHeight - r.bottom;
    list.style.top = below < Math.min(list.offsetHeight, 240) + 8 && r.top > below ? `${r.top - list.offsetHeight - 4}px` : `${r.bottom + 4}px`;
  };
  const close = () => {
    list.remove();
    input.setAttribute('aria-expanded', 'false');
    removeEventListener('scroll', place, true);
  };
  const draw = () => {
    if (!items.length || document.activeElement !== input) return close();
    const footer = opts.footer?.(items);
    patch(list,
      items.map((s, i) =>
        h(`div.menu-item${i === active ? '.active' : ''}`, { role: 'option', 'aria-selected': String(i === active), on: { click: () => pick(i) } },
          s.icon && iconEl(s.icon, 16),
          h('span.suggest-text', h('span', s.label), s.detail && h('span.muted', s.detail)))),
      footer && h('div.suggest-footer', { role: 'presentation' }, footer));
    if (!list.isConnected) {
      document.body.append(list);
      input.setAttribute('aria-expanded', 'true');
      addEventListener('scroll', place, true);
    }
    place();
    list.children[active]?.scrollIntoView({ block: 'nearest' });
  };
  const pick = (i: number) => {
    const s = items[i];
    close();
    if (s) onPick(s);
  };
  const ask = async () => {
    const n = ++asked;
    const found = await source(input.value);
    if (n !== asked) return;
    items = found;
    active = first;
    draw();
  };

  listen(input, 'input', () => void ask());
  listen(input, 'focus', () => void ask());
  listen(input, 'blur', () => { asked++; close(); });
  listen(input, 'keydown', (ev) => {
    const e = ev as KeyboardEvent;
    if (!list.isConnected) return;
    if (e.key === 'ArrowDown') active = (active + 1) % items.length;
    else if (e.key === 'ArrowUp') active = (active <= 0 ? items.length : active) - 1;
    else if (e.key === 'Enter' && active >= 0) {
      e.preventDefault();
      return pick(active);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      return close();
    } else return;
    e.preventDefault();
    draw();
  });
  return { close };
}
