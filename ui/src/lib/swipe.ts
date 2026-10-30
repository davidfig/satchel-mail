// Trackpad paging. A track holding the previous, current and next pages follows
// a horizontal two-finger swipe; when the swipe ends it either springs back or
// slides the neighbouring page fully into view, and the caller re-renders.
// Also: locking a gesture to one axis on something that scrolls both ways.

// Fraction of the page width a swipe must cover to turn the page.
const COMMIT = 0.2;
// Browsers don't report when fingers lift, so a gap in wheel events ends the swipe.
const END_MS = 120;
// ...but on a Mac the trackpad's momentum keeps the wheel events coming long
// after the fingers lift, and browsers don't say which events are momentum.
// Momentum starts at the swipe's top speed and only ever slows (often
// repeating a step), so once the swipe has covered COMMIT, this many steps in a
// row that don't speed up, the last below SLOWED of the top speed, mean the
// fingers have lifted: turn now rather than once the momentum dies away. Only
// after a fast swipe (a step of FLING_PX): a slow drag wobbles in speed with
// the fingers still down, and leaves little momentum to wait out anyway.
const SLOWING_STEPS = 2;
const SLOWED = 0.9;
const FLING_PX = 12;
// After a turn, the swipe's momentum tail is swallowed until the wheel is this quiet.
const QUIET_MS = 200;
const TURN_MS = 220;
const BACK_MS = 200;

const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function move(el: HTMLElement, x: number, ms = 0, ease = 'ease-out') {
  el.style.transition = ms ? `transform ${ms}ms ${ease}` : 'none';
  el.style.transform = x ? `translateX(${x}px)` : '';
}

/** Lets `container` page left and right with a trackpad swipe. `track()` returns
 * the element that moves: as wide as the page, with the previous page laid out
 * just off its left edge and the next just off its right (its parent clips
 * them). `turn(dir)` goes to the next (1) or previous (-1) page and resolves once
 * the new track is rendered. While `enabled()` is false the wheel scrolls natively. */
export function swipePager(container: HTMLElement, track: () => HTMLElement | null | undefined, turn: (dir: 1 | -1) => Promise<unknown>, enabled = () => true) {
  let dx = 0;
  let busy = false;
  let lastWheel = 0;
  let endTimer = 0;
  let lastStep = 0;
  let topStep = 0;
  let slowing = 0;

  const releaseWhenQuiet = () => {
    if (performance.now() - lastWheel >= QUIET_MS) busy = false;
    else setTimeout(releaseWhenQuiet, 50);
  };

  const settle = async () => {
    clearTimeout(endTimer);
    const d = dx;
    dx = 0;
    lastStep = 0;
    topStep = 0;
    slowing = 0;
    const el = track();
    if (!el) return;
    const w = el.clientWidth;
    if (Math.abs(d) < w * COMMIT) {
      move(el, 0, reducedMotion() ? 0 : BACK_MS);
      return;
    }
    const dir = d > 0 ? 1 : -1;
    busy = true;
    try {
      // The neighbour slides fully into view and stays there until the
      // re-render puts it in the middle of a fresh track.
      if (!reducedMotion()) {
        move(el, -dir * w, TURN_MS);
        await wait(TURN_MS);
      }
      await turn(dir);
      // The new page is in the middle of the track now; put the track back.
      move(el, 0);
    } finally {
      releaseWhenQuiet();
    }
  };

  container.addEventListener('wheel', (e) => {
    if (!enabled()) return;
    // Once a swipe has started it owns the wheel, even if it drifts vertical.
    if (!dx && Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    e.preventDefault();
    lastWheel = performance.now();
    if (busy) return;
    const el = track();
    if (!el) return;
    const w = el.clientWidth;
    dx = Math.max(-w, Math.min(w, dx + e.deltaX));
    const step = Math.abs(e.deltaX);
    slowing = step <= lastStep ? slowing + 1 : 0;
    lastStep = step;
    topStep = Math.max(topStep, step);
    const lifted = topStep >= FLING_PX && slowing >= SLOWING_STEPS && step <= topStep * SLOWED;
    if (Math.abs(dx) >= w || (lifted && Math.abs(dx) >= w * COMMIT)) {
      void settle();
      return;
    }
    if (!reducedMotion()) move(el, -dx);
    clearTimeout(endTimer);
    endTimer = window.setTimeout(() => void settle(), END_MS);
  }, { passive: false });
}

// A pause this long between wheel events starts a new gesture.
const GESTURE_GAP_MS = 200;

/** Keeps each trackpad gesture over `scroller()` (inside `container`) to the
 * axis it started on. A sideways swipe drifts up or down a little, mostly as
 * the fingers lift, which would otherwise nudge a grid that scrolls both ways. */
export function lockWheelAxis(container: HTMLElement, scroller: () => HTMLElement | null | undefined) {
  let axis: 'x' | 'y' | null = null;
  let last = 0;
  container.addEventListener('wheel', (e) => {
    const el = scroller();
    if (!el || !el.contains(e.target as Node)) return;
    const now = performance.now();
    if (now - last > GESTURE_GAP_MS) axis = null;
    last = now;
    axis ??= Math.abs(e.deltaX) > Math.abs(e.deltaY) ? 'x' : 'y';
    // Along the axis only: the browser scrolls as usual.
    if (!(axis === 'x' ? e.deltaY : e.deltaX)) return;
    e.preventDefault();
    const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? el.clientHeight : 1;
    if (axis === 'x') el.scrollLeft += e.deltaX * unit;
    else el.scrollTop += e.deltaY * unit;
  }, { passive: false });
}
