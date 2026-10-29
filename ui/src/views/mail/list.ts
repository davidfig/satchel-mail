// The virtualized thread list: scroll geometry and row rendering.

import { h, icon, replace } from '../../lib/dom';
import { listDate, participantsLabel } from '../../lib/format';
import { store } from '../../state';
import type { MailView } from './index';

export const ROW_H = 74;
// Browsers cap element heights (~17M px in WebKit); beyond this the scrollbar is scaled.
export const MAX_SCROLL_PX = 8_000_000;

// Rows live in a logical space of total * ROW_H pixels. When that exceeds what
// browsers can render, the scroll range is compressed by `scale`.

export function scale(view: MailView): number {
  const logical = view.list.total * ROW_H;
  const ch = view.scroller.clientHeight;
  if (logical <= MAX_SCROLL_PX) return 1;
  return (logical - ch) / Math.max(MAX_SCROLL_PX - ch, 1);
}

export function logicalTop(view: MailView): number {
  return view.scroller.scrollTop * view.scale();
}

export function visibleRange(view: MailView, pad = 5): [number, number] {
  const top = view.logicalTop();
  const first = Math.max(0, Math.floor(top / ROW_H) - pad);
  const last = Math.min(view.list.total, Math.ceil((top + view.scroller.clientHeight) / ROW_H) + pad);
  return [first, last];
}

export function scrollToIndex(view: MailView, i: number, offset = 0) {
  view.scroller.scrollTop = (i * ROW_H + offset) / view.scale();
}

export function renderRows(view: MailView) {
  const total = view.list.total;
  view.spacer.style.height = `${Math.max(Math.min(total * ROW_H, MAX_SCROLL_PX), 1)}px`;
  if (!total) {
    replace(view.rows, h('div.empty-list', view.list.loaded ? emptyMessage() : 'Loading…'));
    view.rows.style.transform = '';
    return;
  }
  const [first, last] = view.visibleRange();
  view.list.ensure(first, last);
  // Position the rendered slice relative to the (possibly compressed) scroll offset.
  view.rows.style.transform = `translateY(${view.scroller.scrollTop + (first * ROW_H - view.logicalTop())}px)`;
  const me = store.state.myEmails;
  const out: HTMLElement[] = [];
  for (let idx = first; idx < last; idx++) {
    const t = view.list.at(idx);
    if (!t) {
      out.push(h('div.thread-row.placeholder', { style: { height: `${ROW_H}px` }, 'aria-hidden': 'true' }, h('div.thread-main', h('div.ph.w40'), h('div.ph.w70'), h('div.ph.w90'))));
      continue;
    }
    const active = t.thread_id === store.state.selectedThread;
    out.push(
      h(
        `div.thread-row${t.unread_count ? '.unread' : ''}${active ? '.active' : ''}${idx === view.cursor ? '.cursor' : ''}${view.selected.has(t.thread_id) ? '.selected' : ''}`,
        {
          role: 'option',
          'aria-selected': active ? 'true' : 'false',
          style: { height: `${ROW_H}px` },
          dataset: { idx: String(idx) },
          on: {
            click: (e: MouseEvent) => {
              if (e.metaKey || e.ctrlKey || e.shiftKey) {
                view.toggleSelect(t);
                return;
              }
              view.cursor = idx;
              void view.showThread(t.thread_id);
            },
          },
        },
        h('span.unread-dot', { 'aria-label': t.unread_count ? 'Unread' : undefined }),
        store.state.accounts.length > 1 && h('span.acct-bar', { style: { background: store.accountColor(t.account_id) }, title: store.state.accounts.find((a) => a.id === t.account_id)?.email }),
        h('div.thread-main',
          h('div.thread-line1',
            h('span.who', participantsLabel(t.participants, me)),
            t.message_count > 1 && h('span.count', String(t.message_count)),
            h('span.spacer'),
            t.has_attachments && icon('paperclip', 14),
            h('time', { datetime: t.last_date }, listDate(t.last_date))),
          h('div.thread-subject', t.subject || '(no subject)'),
          h('div.thread-snippet', t.snippet)),
        h('button.star', {
          'aria-label': t.flagged ? 'Unflag' : 'Flag',
          'aria-pressed': t.flagged ? 'true' : 'false',
          on: { click: (e: MouseEvent) => { e.stopPropagation(); void view.flagThread(t); } },
        }, icon('star', 16)),
      ),
    );
  }
  replace(view.rows, out);
}

function emptyMessage(): string {
  const s = store.state.source;
  if (!store.state.accounts.length) return 'No accounts yet — add one in Settings.';
  if (s.kind === 'search') return 'No matching conversations.';
  if (s.kind === 'smart') return 'Nothing in this smart folder right now.';
  if (s.kind === 'person') return s.filter === 'all' ? `No conversations with ${s.label}.` : 'No matching conversations.';
  return s.kind === 'role' && s.role === 'inbox' ? 'Inbox zero. 🎉' : 'Nothing here.';
}
