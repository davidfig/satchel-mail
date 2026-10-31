// Tiny DOM helpers. `h('div.cls#id', {attrs}, ...children)`.

type Child = Node | string | number | null | undefined | false | Child[];
type Attrs = Record<string, unknown> & {
  on?: Record<string, (e: never) => void>;
  style?: Partial<CSSStyleDeclaration> | string;
  dataset?: Record<string, string>;
};

export function h<K extends keyof HTMLElementTagNameMap>(sel: K | `${K}.${string}` | `${K}#${string}`, attrs?: Attrs | Child, ...children: Child[]): HTMLElementTagNameMap[K];
export function h(sel: string, attrs?: Attrs | Child, ...children: Child[]): HTMLElement;
export function h(sel: string, attrs?: Attrs | Child, ...children: Child[]): HTMLElement {
  const m = sel.match(/^([a-z0-9-]+)?((?:[.#][\w-]+)*)$/i);
  const tag = m?.[1] || 'div';
  const el = document.createElement(tag);
  for (const part of (m?.[2] ?? '').match(/[.#][\w-]+/g) ?? []) {
    if (part[0] === '.') el.classList.add(part.slice(1));
    else el.id = part.slice(1);
  }
  if (attrs && typeof attrs === 'object' && !(attrs instanceof Node) && !Array.isArray(attrs)) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'on') {
        for (const [ev, fn] of Object.entries(v as Record<string, EventListener>)) listen(el, ev, fn);
      } else if (k === 'style') {
        if (typeof v === 'string') el.setAttribute('style', v);
        else
          for (const [prop, val] of Object.entries(v as Record<string, string>)) {
            // Custom properties (--x) only work through setProperty.
            if (prop.startsWith('--')) el.style.setProperty(prop, String(val));
            else (el.style as unknown as Record<string, string>)[prop] = String(val);
          }
      } else if (k === 'dataset') {
        Object.assign(el.dataset, v);
      } else if (k === 'title' && tag !== 'iframe') {
        // Styled tooltip instead of the browser's (see tooltip.ts).
        el.dataset.tooltip = String(v);
      } else if (k === 'class') {
        el.className = String(v);
      } else if (k in el && typeof v !== 'string') {
        (el as unknown as Record<string, unknown>)[k] = v;
      } else if (v === true) {
        el.setAttribute(k, '');
      } else {
        el.setAttribute(k, String(v));
      }
    }
  } else if (attrs !== undefined) {
    children.unshift(attrs as Child);
  }
  append(el, children);
  // A tooltip on an icon-only control doubles as its accessible name.
  if (el.dataset.tooltip && !el.hasAttribute('aria-label') && !el.textContent?.trim()) el.setAttribute('aria-label', el.dataset.tooltip);
  return el;
}

export function append(el: Node, children: Child[]) {
  for (const c of children.flat(Infinity as 1) as Child[]) {
    if (c === null || c === undefined || c === false) continue;
    el.appendChild(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

export function clear(el: Element) {
  while (el.firstChild) el.removeChild(el.firstChild);
}

export function replace(el: Element, ...children: Child[]) {
  clear(el);
  append(el, children);
}

/** Listeners added through `listen` (and `h`'s `on`), per element and event.
 *  Each element gets one stable DOM listener per event that calls whatever is
 *  stored here, so `patch` can hand a kept element the new tree's handlers. */
const handlers = new WeakMap<Element, Map<string, EventListener[]>>();

/** addEventListener that `patch` knows about. Use it for listeners on
 *  elements that are re-rendered with `patch`. */
export function listen(el: Element, ev: string, fn: EventListener) {
  let byEvent = handlers.get(el);
  if (!byEvent) handlers.set(el, (byEvent = new Map()));
  let list = byEvent.get(ev);
  if (!list) {
    byEvent.set(ev, (list = []));
    el.addEventListener(ev, (e) => {
      for (const f of handlers.get(el)?.get(ev) ?? []) f.call(el, e);
    });
  }
  list.push(fn);
}

/** Like `replace`, but updates `el`'s existing children in place to match the
 *  new ones instead of swapping them out. Use it for anything that redraws
 *  while it's on screen (sync ticks, scrolling, typing): nodes under the
 *  pointer keep their :hover, tooltips stay up, images don't reload,
 *  animations don't restart.
 *
 *  Render from scratch each time. Kept nodes get the new render's attributes,
 *  text, `listen`/`on` handlers and form state (value, checked, selected; a
 *  focused field keeps what's typed), so:
 *  - attach listeners with `on` or `listen`, never `addEventListener`;
 *  - handlers must not hold on to the elements they were built with (read
 *    `e.currentTarget` instead): those may be thrown away;
 *  - a node you keep yourself (a search box) passed straight in, and already
 *    in `el`, stays exactly as it is.
 *  Forms that hold references to their own fields use `replace`, and don't
 *  redraw while being edited. */
export function patch(el: Element, ...children: Child[]) {
  // Stand-ins hold the kept nodes' places while the new tree is built.
  const kept = new Map<Node, Node>();
  const swap = (c: Child): Child => {
    if (Array.isArray(c)) return c.map(swap);
    if (!(c instanceof Node) || c.parentNode !== el) return c;
    const stand = document.createComment('');
    kept.set(stand, c);
    return stand;
  };
  const next = document.createElement('div');
  append(next, children.map(swap));
  patchChildren(el, next, kept);
}

function patchChildren(old: Node, next: Node, kept = new Map<Node, Node>()) {
  const want = [...next.childNodes];
  const keeping = new Set(kept.values());
  for (let i = 0; i < want.length; i++) {
    const have = old.childNodes[i];
    const mine = kept.get(want[i]);
    if (mine) {
      if (have !== mine) old.insertBefore(mine, have ?? null);
    } else if (have && keeping.has(have)) old.insertBefore(want[i], have);
    else if (have && sameKind(have, want[i])) patchNode(have, want[i]);
    else if (have) old.replaceChild(want[i], have);
    else old.appendChild(want[i]);
  }
  while (old.childNodes.length > want.length) old.removeChild(old.lastChild!);
}

function sameKind(a: Node, b: Node) {
  return a.nodeType === b.nodeType && a.nodeName === b.nodeName;
}

function patchNode(old: Node, next: Node) {
  if (!(old instanceof Element) || !(next instanceof Element)) {
    if (old.nodeValue !== next.nodeValue) old.nodeValue = next.nodeValue;
    return;
  }
  for (const { name } of [...old.attributes]) if (!next.hasAttribute(name)) old.removeAttribute(name);
  for (const { name, value } of [...next.attributes]) if (old.getAttribute(name) !== value) old.setAttribute(name, value);
  const theirs = handlers.get(next);
  for (const ev of handlers.get(old)?.keys() ?? []) if (!theirs?.has(ev)) handlers.get(old)!.set(ev, []);
  for (const [ev, fns] of theirs ?? []) {
    if (!handlers.get(old)?.has(ev)) listen(old, ev, () => {});
    handlers.get(old)!.set(ev, fns);
  }
  patchChildren(old, next);
  patchFormState(old, next);
}

/** Form state lives in properties, not attributes; copy it too. After the
 *  children, so a select's options are already in place. */
function patchFormState(old: Element, next: Element) {
  const o = old as HTMLInputElement;
  const n = next as HTMLInputElement;
  if (old.nodeName === 'INPUT' && o.checked !== n.checked) o.checked = n.checked;
  if ((old.nodeName === 'INPUT' && o.type !== 'file') || old.nodeName === 'TEXTAREA') {
    if (o.value !== n.value && old !== document.activeElement) o.value = n.value;
  }
  if (old.nodeName === 'OPTION') {
    const [os, ns] = [old as HTMLOptionElement, next as HTMLOptionElement];
    if (os.selected !== ns.selected) os.selected = ns.selected;
  }
}

/** Inline SVG icons (stroke-based, 20px grid). */
const ICONS: Record<string, string> = {
  inbox: '<path d="M3 13h4l1.5 2.5h3L13 13h4"/><path d="M4.5 5h11L18 13v3.5a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V13z"/>',
  send: '<path d="M3 10l14-6-5 13-2.5-5.5z"/><path d="M9.5 11.5L17 4"/>',
  draft: '<path d="M4 16l1-4 8.5-8.5a1.4 1.4 0 0 1 2 0l1 1a1.4 1.4 0 0 1 0 2L8 15z"/><path d="M12 5l3 3"/>',
  archive: '<rect x="2.5" y="3.5" width="15" height="4" rx="1"/><path d="M4 7.5v8a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1v-8"/><path d="M8 11h4"/>',
  trash: '<path d="M3.5 5.5h13"/><path d="M8 5.5V4a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v1.5"/><path d="M5 5.5l.8 10.6a1 1 0 0 0 1 .9h6.4a1 1 0 0 0 1-.9L15 5.5"/>',
  junk: '<circle cx="10" cy="10" r="7"/><path d="M5 15L15 5"/>',
  folder: '<path d="M2.5 6a1 1 0 0 1 1-1h4l1.5 1.5h7.5a1 1 0 0 1 1 1V15a1 1 0 0 1-1 1h-13a1 1 0 0 1-1-1z"/>',
  label: '<path d="M3 4.5a1 1 0 0 1 1-1h6.5l6 6.5-6 6.5H4a1 1 0 0 1-1-1z"/><circle cx="7" cy="8" r="1"/>',
  star: '<path d="M10 2.8l2.2 4.6 5 .6-3.7 3.4 1 5-4.5-2.5-4.5 2.5 1-5L2.8 8l5-.6z"/>',
  calendar: '<rect x="2.5" y="4" width="15" height="13" rx="1.5"/><path d="M2.5 8h15M6.5 2.5v3M13.5 2.5v3"/>',
  review: '<path d="M4 3.5h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-5l-4 3v-3H4a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z"/><path d="M7 8.5l2 2 4-4"/>',
  settings: '<circle cx="10" cy="10" r="2.5"/><path d="M10 2v2M10 16v2M2 10h2M16 10h2M4.3 4.3l1.4 1.4M14.3 14.3l1.4 1.4M4.3 15.7l1.4-1.4M14.3 5.7l1.4-1.4"/>',
  search: '<circle cx="8.5" cy="8.5" r="5"/><path d="M12.5 12.5L17 17"/>',
  compose: '<path d="M10 4H4.5a1 1 0 0 0-1 1v10.5a1 1 0 0 0 1 1H15a1 1 0 0 0 1-1V10"/><path d="M9 11l.5-2.5 6-6a1.2 1.2 0 0 1 1.7 1.7l-6 6z"/>',
  reply: '<path d="M8 5L3 10l5 5"/><path d="M3 10h8a6 6 0 0 1 6 6"/>',
  replyAll: '<path d="M9 5L4 10l5 5"/><path d="M6 5L1 10l5 5" transform="translate(0)"/><path d="M4 10h7a6 6 0 0 1 6 6"/>',
  forward: '<path d="M12 5l5 5-5 5"/><path d="M17 10H9a6 6 0 0 0-6 6"/>',
  mailOpen: '<path d="M2.5 8l7.5-5 7.5 5v8a1 1 0 0 1-1 1h-13a1 1 0 0 1-1-1z"/><path d="M2.5 8l7.5 5 7.5-5"/>',
  mail: '<rect x="2.5" y="4.5" width="15" height="11" rx="1"/><path d="M2.5 5.5l7.5 5.5 7.5-5.5"/>',
  paperclip: '<path d="M15 9.5l-5.6 5.6a3.5 3.5 0 0 1-5-5L10.6 4a2.3 2.3 0 0 1 3.3 3.3L7.8 13.4a1.2 1.2 0 0 1-1.7-1.7l5.6-5.6"/>',
  close: '<path d="M5 5l10 10M15 5L5 15"/>',
  check: '<path d="M4 10.5l4 4 8-9"/>',
  chevronLeft: '<path d="M12.5 4.5L7 10l5.5 5.5"/>',
  chevronRight: '<path d="M7.5 4.5L13 10l-5.5 5.5"/>',
  chevronDown: '<path d="M4.5 7.5L10 13l5.5-5.5"/>',
  plus: '<path d="M10 4v12M4 10h12"/>',
  refresh: '<path d="M16 10a6 6 0 1 1-1.8-4.3"/><path d="M16 3.5v3.5h-3.5"/>',
  sparkle: '<path d="M10 2.5l1.6 4.4 4.4 1.6-4.4 1.6L10 14.5l-1.6-4.4L4 8.5l4.4-1.6z"/><path d="M15.5 13.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z"/>',
  warning: '<path d="M10 3l8 14H2z"/><path d="M10 8.5v4M10 14.5v.5"/>',
  user: '<circle cx="10" cy="7" r="3.5"/><path d="M3.5 17a6.5 6.5 0 0 1 13 0"/>',
  bell: '<path d="M5 13.5V9a5 5 0 0 1 10 0v4.5l1.5 2h-13z"/><path d="M8.5 17.5a1.5 1.5 0 0 0 3 0"/>',
  clock: '<circle cx="10" cy="10" r="7"/><path d="M10 6v4l2.5 2.5"/>',
  location: '<path d="M10 17.5s5.5-5 5.5-9a5.5 5.5 0 0 0-11 0c0 4 5.5 9 5.5 9z"/><circle cx="10" cy="8.5" r="2"/>',
  image: '<rect x="2.5" y="3.5" width="15" height="13" rx="1.5"/><circle cx="7" cy="8" r="1.5"/><path d="M2.5 14l4-4 3 3 3-3 5 5"/>',
  more: '<circle cx="5" cy="10" r="1"/><circle cx="10" cy="10" r="1"/><circle cx="15" cy="10" r="1"/>',
  lock: '<rect x="4.5" y="9" width="11" height="8" rx="1.5"/><path d="M7 9V6.5a3 3 0 0 1 6 0V9"/>',
  key: '<circle cx="6.5" cy="13.5" r="3.5"/><path d="M9 11l7-7M14 6l2 2M12 8l1.5 1.5"/>',
  shield: '<path d="M10 2.5l6 2.5v4.5c0 4-2.6 6.8-6 8-3.4-1.2-6-4-6-8V5z"/>',
  sun: '<circle cx="10" cy="10" r="3.5"/><path d="M10 2v1.5M10 16.5V18M2 10h1.5M16.5 10H18M4.3 4.3l1 1M14.7 14.7l1 1M4.3 15.7l1-1M14.7 5.3l1-1"/>',
  moon: '<path d="M16 12.5A6.5 6.5 0 0 1 7.5 4a6.5 6.5 0 1 0 8.5 8.5z"/>',
  notes: '<path d="M4 5h12M4 10h12M4 15h7.5"/>',
  heart: '<path d="M10 16.5s-6.5-3.8-6.5-8.4A3.4 3.4 0 0 1 10 6a3.4 3.4 0 0 1 6.5 2.1c0 4.6-6.5 8.4-6.5 8.4z"/>',
  download: '<path d="M10 3v10M5.5 8.5L10 13l4.5-4.5"/><path d="M3.5 16.5h13"/>',
  external: '<path d="M11 3.5h5.5V9"/><path d="M16.5 3.5L9 11"/><path d="M14 11.5v4a1 1 0 0 1-1 1H4.5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h4"/>',
  expand: '<path d="M11.5 3.5h5v5M8.5 16.5h-5v-5"/><path d="M16.5 3.5L11 9M3.5 16.5L9 11"/>',
  collapse: '<path d="M16.5 8.5h-5v-5M3.5 11.5h5v5"/><path d="M11.5 8.5L17 3M8.5 11.5L3 17"/>',
  file: '<path d="M5 2.5h6.5l4 4v10a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1z"/><path d="M11.5 2.5v4h4"/>',
  copy: '<rect x="7" y="7" width="10" height="10" rx="1.5"/><path d="M13 7V4.5a1 1 0 0 0-1-1H4.5a1 1 0 0 0-1 1V12a1 1 0 0 0 1 1H7"/>',
};

export function icon(name: keyof typeof ICONS | string, size = 18): SVGElement {
  const wrap = document.createElement('span');
  wrap.innerHTML = `<svg class="icon" width="${size}" height="${size}" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] ?? ''}</svg>`;
  return wrap.firstElementChild as SVGElement;
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t: number | undefined;
  return (...a: A) => {
    clearTimeout(t);
    t = window.setTimeout(() => fn(...a), ms);
  };
}
