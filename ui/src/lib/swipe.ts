// Trackpad paging. A track holding the previous, current and next pages follows
// a horizontal two-finger swipe; when the swipe ends it either springs back or
// slides the neighbouring page fully into view, and the caller re-renders.

// Fraction of the page width a swipe must cover to turn the page.
const COMMIT = 0.2;
// Browsers don't report when fingers lift, so a gap in wheel events ends the swipe.
const END_MS = 120;
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
 * the new track is rendered. */
export function swipePager(container: HTMLElement, track: () => HTMLElement | null | undefined, turn: (dir: 1 | -1) => Promise<unknown>) {
  let dx = 0;
  let busy = false;
  let lastWheel = 0;
  let endTimer = 0;

  const releaseWhenQuiet = () => {
    if (performance.now() - lastWheel >= QUIET_MS) busy = false;
    else setTimeout(releaseWhenQuiet, 50);
  };

  const settle = async () => {
    clearTimeout(endTimer);
    const d = dx;
    dx = 0;
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
    } finally {
      releaseWhenQuiet();
    }
  };

  container.addEventListener('wheel', (e) => {
    // Once a swipe has started it owns the wheel, even if it drifts vertical.
    if (!dx && Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    e.preventDefault();
    lastWheel = performance.now();
    if (busy) return;
    const el = track();
    if (!el) return;
    const w = el.clientWidth;
    dx = Math.max(-w, Math.min(w, dx + e.deltaX));
    if (Math.abs(dx) >= w) {
      void settle();
      return;
    }
    if (!reducedMotion()) move(el, -dx);
    clearTimeout(endTimer);
    endTimer = window.setTimeout(() => void settle(), END_MS);
  }, { passive: false });
}
