// Toolbar overflow: when a row no longer fits, its trailing items are hidden
// and offered from a "…" button instead.

import { h, icon } from './dom';
import { popupMenu } from './ui';
import type { MenuItem } from './ui';

/** Keep `row` on one line by moving `items` (from the end) into a "…" menu
 *  when the row, or a truncating `watch` element in it, overflows. A `pinned`
 *  item (say, the active tab) goes last. Returns the "…" button, to be
 *  placed in the row where the menu should open. */
export function overflowMenu(row: HTMLElement, items: { el: HTMLElement; item: () => MenuItem; pinned?: boolean }[], watch: HTMLElement[] = []): HTMLButtonElement {
  let hidden: typeof items = [];
  const more = h('button.icon-btn.overflow-more', {
    title: 'More',
    'aria-label': 'More',
    on: { click: () => popupMenu(more, hidden.map((i) => i.item())) },
  }, icon('more'));
  const overflowing = () => row.scrollWidth > row.clientWidth + 1 || watch.some((w) => w.scrollWidth > w.clientWidth + 1);
  const fit = () => {
    if (!row.isConnected) return observer.disconnect();
    for (const i of items) i.el.classList.remove('overflowed');
    more.classList.add('overflowed');
    hidden = [];
    if (!overflowing()) return;
    more.classList.remove('overflowed');
    const order = [...items.filter((i) => i.pinned), ...items.filter((i) => !i.pinned)];
    for (let n = order.length - 1; n >= 0 && overflowing(); n--) {
      order[n].el.classList.add('overflowed');
      hidden.push(order[n]);
    }
    hidden.sort((a, b) => items.indexOf(a) - items.indexOf(b));
  };
  const observer = new ResizeObserver(fit);
  observer.observe(row);
  return more;
}
