// Shared UI pieces: toasts, modals, popover menus, keyboard shortcuts.

import { h, icon, listen } from './dom';

// ---------------------------------------------------------------- toasts

interface ToastOpts {
  kind?: 'info' | 'error' | 'success';
  action?: { label: string; run: () => void };
  ms?: number;
  /** Only confirms what the screen already shows: flashes briefly and never replaces an error. */
  quiet?: boolean;
}

interface Slot {
  el: HTMLElement;
  message: string;
  kind: string;
  count: number;
  label: HTMLElement;
  timers: number[];
}

// At most two toasts show: one with an action (Undo and the like), which only another action
// toast replaces, and one plain toast, which any new plain toast replaces. Repeats of the same
// message count up instead of stacking.
let toastHost: HTMLElement | null = null;
const slots: { action: Slot | null; plain: Slot | null } = { action: null, plain: null };

function clearSlot(key: 'action' | 'plain') {
  const s = slots[key];
  if (!s) return;
  s.timers.forEach(clearTimeout);
  s.el.remove();
  slots[key] = null;
}

export function toast(message: string, opts: ToastOpts = {}) {
  const kind = opts.kind ?? 'info';
  const key = opts.action ? 'action' : 'plain';
  const ms = opts.ms ?? (opts.quiet ? 1800 : 4500);
  const cur = slots[key];
  if (key === 'plain' && opts.quiet && cur?.kind === 'error') return;

  const arm = (s: Slot) => {
    s.timers.forEach(clearTimeout);
    s.el.classList.remove('leaving');
    s.timers = [window.setTimeout(() => s.el.classList.add('leaving'), ms - 300), window.setTimeout(() => clearSlot(key), ms)];
  };

  if (cur && cur.message === message && cur.kind === kind && !opts.action) {
    cur.count++;
    cur.label.textContent = `${message} ×${cur.count}`;
    arm(cur);
    return;
  }
  clearSlot(key);
  if (!toastHost) {
    toastHost = h('div.toasts', { role: 'status', 'aria-live': 'polite' });
    document.body.appendChild(toastHost);
  }
  const label = h('span', message);
  const el = h(
    `div.toast.${kind}`,
    label,
    opts.action &&
      h('button.link', {
        on: {
          click: () => {
            opts.action!.run();
            clearSlot(key);
          },
        },
      }, opts.action.label),
    h('button.icon-btn', { 'aria-label': 'Dismiss', on: { click: () => clearSlot(key) } }, icon('close', 14)),
  );
  const slot: Slot = { el, message, kind, count: 1, label, timers: [] };
  slots[key] = slot;
  // The action toast sits below the plain one, nearest the edge.
  if (key === 'action') toastHost.appendChild(el);
  else toastHost.insertBefore(el, toastHost.firstChild);
  arm(slot);
}

export function errorToast(e: unknown) {
  toast(e instanceof Error ? e.message : String(e), { kind: 'error', ms: 7000 });
}

/** Puts `text` on the clipboard and says so (`done` is the toast). */
export function copyText(text: string, done = 'Copied') {
  void navigator.clipboard.writeText(text).then(() => toast(done, { quiet: true }), errorToast);
}

/** Text to paste elsewhere (a token, a command, a prompt) with a Copy button beside it. */
export function copyBlock(text: string, tag = 'pre.snippet'): HTMLElement {
  return h('div.copy-row', h(tag, text), h('button', { title: 'Copy to clipboard', on: { click: () => copyText(text) } }, icon('copy', 16), 'Copy'));
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
    // An open menu or suggestion list closes first, and only the top dialog
    // closes when one is open over another.
    if (e.key === 'Escape' && !document.querySelector('.menu') && root === [...document.querySelectorAll('.modal-backdrop')].pop()) {
      e.stopPropagation();
      close();
    }
  };
  const panel = h(
    `div.modal${opts.wide ? '.wide' : ''}`,
    { role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
    h('header.modal-head', h('h2', title), h('button.icon-btn', { 'aria-label': 'Close', title: 'Close (Esc)', on: { click: close } }, icon('close'))),
    body,
    opts.footer && h('footer.modal-foot', opts.footer),
  );
  const root = h('div.modal-backdrop', { on: { mousedown: (e: MouseEvent) => e.target === root && close() } }, panel);
  document.body.appendChild(root);
  document.addEventListener('keydown', onKey, true);
  requestAnimationFrame(() => (panel.querySelector('input,textarea,select,button.primary') as HTMLElement | null)?.focus());
  return { close, body, root };
}

/** `message` is plain text, or a node for a richer body. */
export function confirmDialog(title: string, message: string | Node, okLabel = 'OK', danger = false): Promise<boolean> {
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
    const m = modal(title, typeof message === 'string' ? h('p', message) : message, { footer, onClose: () => finish(false) });
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
const bindings = new Map<string, { scope: string; fn: Handler; desc: string; when?: () => boolean }[]>();
let activeScope = 'mail';

export function setScope(s: string) {
  activeScope = s;
}

/** Register a key like "j", "shift+3", "mod+k", "g i". `when` says whether
 *  it has anything to act on, so the command palette can leave it out. */
export function bind(key: string, desc: string, fn: Handler, scope = 'global', when?: () => boolean) {
  const list = bindings.get(key) ?? [];
  list.push({ scope, fn, desc, when });
  bindings.set(key, list);
}

/** How a key reads on screen: ⌘K on a Mac (Ctrl+K elsewhere), Shift+I. */
export function keyLabel(key: string): string {
  const mac = navigator.platform.includes('Mac');
  return key
    .replace(/mod\+(.)$/, (_, c: string) => (mac ? '⌘' : 'Ctrl+') + c.toUpperCase())
    .replace(/shift\+(.)$/, (_, c: string) => `Shift+${c.toUpperCase()}`);
}

/** Something the command palette can run. */
export interface Command {
  label: string;
  run: () => void;
  /** Its shortcut, as `keyLabel` shows it. */
  hint?: string;
  /** Secondary text, such as the account a folder belongs to. */
  detail?: string;
  icon?: string;
}

/** Commands a view adds to the palette, built when it opens, and what they
 *  act on ("“Lunch on Friday”"), shown above them. */
export type CommandSource = () => { context?: string; commands: Command[] };
const sources: { scope: string; fn: CommandSource }[] = [];

export function commandSource(fn: CommandSource, scope = 'global') {
  sources.push({ scope, fn });
}

/** Keys that move around a list or close things; not worth a palette entry. */
const NOT_COMMANDS = new Set(['ArrowDown', 'ArrowUp', 'shift+ArrowDown', 'shift+ArrowUp', 'ArrowLeft', 'ArrowRight', 'Enter', 'Escape', 'Backspace', 'Delete', 'j', 'k', 'o', 'x', 'mod+k']);

/** What the palette offers now: the current view's shortcuts and commands,
 *  then the global ones. A shortcut's command shows its key. */
export function activeCommands(): { context?: string; commands: Command[] } {
  const out: Command[] = [];
  const seen = new Set<string>();
  const add = (c: Command) => {
    if (seen.has(c.label)) return;
    seen.add(c.label);
    out.push(c);
  };
  let context: string | undefined;
  for (const scope of [activeScope, 'global']) {
    for (const key of bindings.keys()) {
      const b = binding(key);
      if (b?.scope !== scope || NOT_COMMANDS.has(key) || (b.when && !b.when())) continue;
      add({ label: b.desc, hint: keyLabel(key), run: () => b.fn(new KeyboardEvent('keydown')) });
    }
    for (const s of sources.filter((x) => x.scope === scope)) {
      const r = s.fn();
      context ??= r.context;
      r.commands.forEach(add);
    }
  }
  return { context, commands: out };
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
  if (e.shiftKey && ((e.key.length === 1 && /[a-z]/i.test(e.key)) || e.key.startsWith('Arrow'))) mods.push('shift');
  if (k === ' ') k = 'space';
  return [...mods, k].join('+');
}

/** The handler `key` runs in the current view, if any. */
function binding(key: string) {
  return bindings.get(key)?.filter((b) => b.scope === 'global' || b.scope === activeScope).pop();
}

/** Runs what pressing `key` would, for menus that offer the same action.
 *  Like the keyboard, plain keys do nothing while a dialog is open. */
export function trigger(key: string) {
  if (document.querySelector('.modal-backdrop') && !key.startsWith('mod+')) return;
  binding(key)?.fn(new KeyboardEvent('keydown'));
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
      const b = binding(key);
      if (!b) return false;
      e.preventDefault();
      b.fn(e);
      return true;
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
  listen(el, 'contextmenu', (ev) => {
    const e = ev as MouseEvent;
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
