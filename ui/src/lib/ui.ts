// Shared UI pieces: toasts, modals, popover menus, keyboard shortcuts.

import { h, icon } from './dom';

// ---------------------------------------------------------------- toasts

let toastHost: HTMLElement | null = null;

export function toast(message: string, opts: { kind?: 'info' | 'error' | 'success'; action?: { label: string; run: () => void }; ms?: number } = {}) {
  if (!toastHost) {
    toastHost = h('div.toasts', { role: 'status', 'aria-live': 'polite' });
    document.body.appendChild(toastHost);
  }
  const t = h(
    `div.toast.${opts.kind ?? 'info'}`,
    h('span', message),
    opts.action &&
      h('button.link', {
        on: {
          click: () => {
            opts.action!.run();
            t.remove();
          },
        },
      }, opts.action.label),
    h('button.icon-btn', { 'aria-label': 'Dismiss', on: { click: () => t.remove() } }, icon('close', 14)),
  );
  toastHost.appendChild(t);
  setTimeout(() => t.classList.add('leaving'), (opts.ms ?? 4500) - 300);
  setTimeout(() => t.remove(), opts.ms ?? 4500);
}

export function errorToast(e: unknown) {
  toast(e instanceof Error ? e.message : String(e), { kind: 'error', ms: 7000 });
}

// ---------------------------------------------------------------- modal

export interface ModalHandle {
  close: () => void;
  body: HTMLElement;
  root: HTMLElement;
}

export function modal(title: string, content: Node, opts: { wide?: boolean; onClose?: () => void; footer?: Node } = {}): ModalHandle {
  const prev = document.activeElement as HTMLElement | null;
  const body = h('div.modal-body', content);
  const close = () => {
    root.remove();
    document.removeEventListener('keydown', onKey, true);
    opts.onClose?.();
    prev?.focus?.();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  };
  const panel = h(
    `div.modal${opts.wide ? '.wide' : ''}`,
    { role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
    h('header.modal-head', h('h2', title), h('button.icon-btn', { 'aria-label': 'Close', on: { click: close } }, icon('close'))),
    body,
    opts.footer && h('footer.modal-foot', opts.footer),
  );
  const root = h('div.modal-backdrop', { on: { mousedown: (e: MouseEvent) => e.target === root && close() } }, panel);
  document.body.appendChild(root);
  document.addEventListener('keydown', onKey, true);
  requestAnimationFrame(() => (panel.querySelector('input,textarea,select,button.primary') as HTMLElement | null)?.focus());
  return { close, body, root };
}

export function confirmDialog(title: string, message: string, okLabel = 'OK', danger = false): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v: boolean) => {
      if (done) return;
      done = true;
      m.close();
      resolve(v);
    };
    const footer = h(
      'div.row.end',
      h('button', { on: { click: () => finish(false) } }, 'Cancel'),
      h(`button.primary${danger ? '.danger' : ''}`, { on: { click: () => finish(true) } }, okLabel),
    );
    const m = modal(title, h('p', message), { footer, onClose: () => finish(false) });
  });
}

// ---------------------------------------------------------------- menus

export interface MenuItem {
  label: string;
  icon?: string;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  run: () => void;
}

/** Open a menu below `anchor`, or at a point (a right-click). */
export function popupMenu(anchor: HTMLElement | { x: number; y: number }, items: (MenuItem | '-')[]) {
  document.querySelector('.menu')?.remove();
  const menu = h(
    'div.menu',
    { role: 'menu' },
    items.map((it) =>
      it === '-'
        ? h('div.menu-sep')
        : h(
            `button.menu-item${it.danger ? '.danger' : ''}`,
            {
              role: 'menuitem',
              disabled: it.disabled,
              on: {
                click: () => {
                  menu.remove();
                  it.run();
                },
              },
            },
            it.icon && icon(it.icon, 16),
            h('span', it.label),
            it.hint && h('kbd', it.hint),
          ),
    ),
  );
  document.body.appendChild(menu);
  const r = anchor instanceof HTMLElement ? anchor.getBoundingClientRect() : { left: anchor.x, top: anchor.y, bottom: anchor.y };
  const mw = menu.offsetWidth;
  const mh = menu.offsetHeight;
  menu.style.left = `${Math.min(r.left, innerWidth - mw - 8)}px`;
  menu.style.top = `${r.bottom + mh + 4 > innerHeight ? r.top - mh - 4 : r.bottom + 4}px`;
  const off = (e: MouseEvent) => {
    if (!menu.contains(e.target as Node)) {
      menu.remove();
      document.removeEventListener('mousedown', off, true);
    }
  };
  setTimeout(() => document.addEventListener('mousedown', off, true));
  (menu.querySelector('button') as HTMLElement | null)?.focus();
  menu.addEventListener('keydown', (e) => {
    const btns = [...menu.querySelectorAll<HTMLButtonElement>('button:not([disabled])')];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'ArrowDown') btns[(i + 1) % btns.length]?.focus();
    else if (e.key === 'ArrowUp') btns[(i - 1 + btns.length) % btns.length]?.focus();
    else if (e.key === 'Escape') menu.remove();
    else return;
    e.preventDefault();
  });
}

// ---------------------------------------------------------------- shortcuts

type Handler = (e: KeyboardEvent) => void;
const bindings = new Map<string, { scope: string; fn: Handler; desc: string }[]>();
let activeScope = 'mail';

export function setScope(s: string) {
  activeScope = s;
}

/** Register a key like "j", "shift+3", "mod+k", "g i". */
export function bind(key: string, desc: string, fn: Handler, scope = 'global') {
  const list = bindings.get(key) ?? [];
  list.push({ scope, fn, desc });
  bindings.set(key, list);
}

export function shortcutList(): { key: string; desc: string; scope: string }[] {
  const out: { key: string; desc: string; scope: string }[] = [];
  for (const [key, list] of bindings) for (const b of list) out.push({ key, desc: b.desc, scope: b.scope });
  return out;
}

let pendingPrefix: string | null = null;
let prefixTimer: number | undefined;

function keyName(e: KeyboardEvent): string {
  const mods = [];
  if (e.metaKey || e.ctrlKey) mods.push('mod');
  if (e.altKey) mods.push('alt');
  let k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (e.shiftKey && e.key.length === 1 && /[a-z]/i.test(e.key)) mods.push('shift');
  if (k === ' ') k = 'space';
  return [...mods, k].join('+');
}

export function installShortcuts() {
  document.addEventListener('keydown', (e) => {
    const t = e.target as HTMLElement;
    const typing = t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName);
    const k = keyName(e);
    if (typing && !k.startsWith('mod+') && k !== 'Escape') return;
    if (document.querySelector('.modal-backdrop') && !k.startsWith('mod+')) return;
    const full = pendingPrefix ? `${pendingPrefix} ${k}` : k;
    const run = (key: string) => {
      const list = bindings.get(key)?.filter((b) => b.scope === 'global' || b.scope === activeScope);
      if (list?.length) {
        e.preventDefault();
        list[list.length - 1].fn(e);
        return true;
      }
      return false;
    };
    if (pendingPrefix) {
      pendingPrefix = null;
      clearTimeout(prefixTimer);
      if (run(full)) return;
    }
    if (k === 'g' && !typing) {
      pendingPrefix = 'g';
      prefixTimer = window.setTimeout(() => (pendingPrefix = null), 1200);
      return;
    }
    run(k);
  });
}

/** Our own right-click menu on `el`, built when it opens. Text selections,
 *  links and form fields inside keep the system menu, so Copy still works. */
export function contextMenu(el: HTMLElement, build: (e: MouseEvent) => (MenuItem | '-')[] | null) {
  el.addEventListener('contextmenu', (e) => {
    const t = e.target as HTMLElement;
    if (t.closest('a, input, textarea, [contenteditable="true"]') || !(getSelection()?.isCollapsed ?? true)) return;
    const items = build(e);
    if (!items?.length) return;
    e.preventDefault();
    e.stopPropagation();
    // The keyboard's menu key (or Shift+F10) reports no pointer position.
    const at = e.clientX || e.clientY ? { x: e.clientX, y: e.clientY } : el;
    popupMenu(at, items);
  });
}
