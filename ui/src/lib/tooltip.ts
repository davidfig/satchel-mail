// App-wide tooltips. One delegated controller shows a styled tooltip for any
// element with `data-tooltip`, and for every button (falling back to its
// aria-label, then its text). `h()` turns a `title` attr into `data-tooltip`, and
// any native `title` that slips through is converted on hover, so the browser's
// own tooltip never shows.

const SHOW_DELAY = 450;
// Moving straight from one tooltip to the next skips the delay.
const WARM_MS = 300;
const GAP = 6;
const EDGE = 8;

let tip: HTMLElement | null = null;
let target: HTMLElement | null = null;
let showTimer: number | undefined;
let watch: number | undefined;
let lastHidden = 0;

/** The tooltip text for `el`, or '' if it has none. */
function textFor(el: HTMLElement): string {
  const own = el.dataset.tooltip;
  if (own !== undefined) return own;
  if (el.tagName !== 'BUTTON') return '';
  return el.getAttribute('aria-label') ?? el.textContent?.trim().replace(/\s+/g, ' ') ?? '';
}

/** The nearest element (from the event target up) that has a tooltip. */
function findTarget(from: EventTarget | null): HTMLElement | null {
  let el = from instanceof Element ? from : null;
  while (el && el !== document.body) {
    if (el instanceof HTMLElement) {
      // Take over native titles before the browser can show them.
      const native = el.getAttribute('title');
      if (native !== null && el.tagName !== 'IFRAME') {
        el.removeAttribute('title');
        el.dataset.tooltip = native;
      }
      if (el.dataset.tooltip !== undefined || el.tagName === 'BUTTON') return textFor(el) ? el : null;
    }
    el = el.parentElement;
  }
  return null;
}

function place(el: HTMLElement, t: HTMLElement) {
  const r = el.getBoundingClientRect();
  const w = t.offsetWidth;
  const ht = t.offsetHeight;
  let top = r.bottom + GAP;
  let side = 'below';
  if (top + ht > window.innerHeight - EDGE) {
    top = r.top - GAP - ht;
    side = 'above';
  }
  const left = Math.min(Math.max(r.left + r.width / 2 - w / 2, EDGE), window.innerWidth - w - EDGE);
  t.style.left = `${Math.round(left)}px`;
  t.style.top = `${Math.round(Math.max(top, EDGE))}px`;
  t.dataset.side = side;
}

function show(el: HTMLElement) {
  const text = textFor(el);
  if (!text || !el.isConnected) return;
  if (!tip) {
    tip = document.createElement('div');
    tip.className = 'tooltip';
    tip.id = 'app-tooltip';
    tip.setAttribute('role', 'tooltip');
    document.body.appendChild(tip);
  }
  tip.textContent = text;
  tip.hidden = false;
  place(el, tip);
  // Icon buttons already have an aria-label; describe the rest.
  if (!el.hasAttribute('aria-label')) el.setAttribute('aria-describedby', tip.id);
  clearInterval(watch);
  // Views re-render by replacing nodes, which fires no pointerout.
  watch = window.setInterval(() => {
    if (!target?.isConnected) hide();
  }, 250);
}

function hide() {
  clearTimeout(showTimer);
  clearInterval(watch);
  if (target?.getAttribute('aria-describedby') === 'app-tooltip') target.removeAttribute('aria-describedby');
  if (tip && !tip.hidden) {
    tip.hidden = true;
    lastHidden = Date.now();
  }
  target = null;
}

function enter(el: HTMLElement | null) {
  if (el === target) return;
  hide();
  if (!el) return;
  target = el;
  const warm = Date.now() - lastHidden < WARM_MS;
  showTimer = window.setTimeout(() => show(el), warm ? 0 : SHOW_DELAY);
}

/** Installs the tooltip controller. Call once at startup. */
export function installTooltips() {
  document.addEventListener('pointerover', (e) => {
    if ((e as PointerEvent).pointerType === 'touch') return;
    enter(findTarget(e.target));
  });
  document.addEventListener('pointerout', (e) => {
    const to = (e as PointerEvent).relatedTarget;
    if (target && !(to instanceof Node && target.contains(to))) hide();
  });
  document.addEventListener('focusin', (e) => {
    // Only for keyboard focus, so clicking a button doesn't pin its tooltip.
    const el = e.target instanceof HTMLElement && e.target.matches(':focus-visible') ? findTarget(e.target) : null;
    enter(el);
  });
  document.addEventListener('focusout', hide);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('keydown', (e) => e.key === 'Escape' && hide(), true);
  window.addEventListener('scroll', hide, true);
  window.addEventListener('blur', hide);
}
