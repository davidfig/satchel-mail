// Pure thread helpers: quote detection and which messages to fold. No app imports,
// so they run under `node --test` (see ui/test/).

import type { Address, MessageDetail } from '../types';

/** Show the first message, a fold, then this many before the first new one. */
const CONTEXT_BEFORE_NEW = 1;
/** Always show at least this many of the latest messages. */
const MIN_RECENT = 3;
/** Folding fewer messages than this isn't worth a click. */
const MIN_FOLD = 2;

export const isAttribution = (l: string) => /^On .+wrote:\s*$/.test(l.trim()) || /^-{2,} ?(Original|Forwarded) [Mm]essage/.test(l.trim());

export interface FoldPlan {
  /** Index of the first unread message from someone else, or -1. */
  firstNew: number;
  /** Messages in [start, end) are folded behind "N earlier messages"; null when nothing is. */
  fold: { start: number; end: number } | null;
}

/** Keeps the first message, the latest few, and one message of context before the first new one. */
export function planFold(msgs: Pick<MessageDetail, 'flags' | 'from'>[], mine: (a: Address | null | undefined) => boolean): FoldPlan {
  const firstNew = msgs.findIndex((m) => !m.flags.seen && !mine(m.from));
  const focus = firstNew >= 0 ? firstNew : msgs.length - 1;
  const end = Math.max(1, Math.min(focus - CONTEXT_BEFORE_NEW, msgs.length - MIN_RECENT));
  return { firstNew, fold: end - 1 >= MIN_FOLD ? { start: 1, end } : null };
}

/** Newsletters, receipts and other laid-out mail keep their formatting instead of becoming a bubble. */
export function looksDesigned(html: string): boolean {
  return /<table\b/i.test(html) || (html.match(/<img\b/gi)?.length ?? 0) >= 2;
}

/**
 * Cuts quoted history out of (already sanitized) HTML: Gmail, Apple Mail, Outlook,
 * Thunderbird and Yahoo markers. `quoted` is the original, for "show quoted text".
 */
export function stripHtmlQuotes(html: string): { html: string; quoted: string | null } {
  // An explicit body so every parser (browsers, linkedom in tests) builds the same tree.
  const doc = new DOMParser().parseFromString(`<!doctype html><html><head></head><body>${html}</body></html>`, 'text/html');
  const cut =
    doc.querySelector('.gmail_quote, #appendonsend, #divRplyFwdMsg, .moz-cite-prefix, .yahoo_quoted, div[id^="yahoo_quoted"], blockquote[type="cite"]') ??
    outlookHeader(doc);
  if (!cut) return { html, quoted: null };
  // Apple Mail puts "On …, X wrote:" in the element just before the blockquote.
  const before = cut.previousElementSibling;
  const start = before && (before.tagName === 'HR' || isAttribution(before.textContent ?? '')) ? before : cut;
  removeFrom(start, doc.body);
  if (!doc.body.textContent?.trim() && !doc.body.querySelector('img')) return { html, quoted: null };
  return { html: doc.body.innerHTML, quoted: html };
}

/** Outlook's plain "From: … Sent: … To: … Subject:" header block. */
function outlookHeader(doc: Document): Element | null {
  for (const el of doc.body.querySelectorAll('div, p')) {
    const t = (el.textContent ?? '').trim();
    // No \b: lines are split by <br>, so textContent runs them together ("BobSubject:").
    if (t.length < 600 && /^From:/.test(t) && /(Sent|Date):/.test(t) && /Subject:/.test(t)) return el;
  }
  return null;
}

/** Removes `el` and everything after it in document order. */
function removeFrom(el: Element, root: Element) {
  let node: Element | null = el;
  while (node && node !== root) {
    while (node.nextSibling) node.nextSibling.remove();
    const parent: Element | null = node.parentElement;
    if (node === el) node.remove();
    node = parent;
  }
}

/** Splits a reply body into what the writer typed and the quoted history below it. */
export function splitQuote(text: string): [string, string] {
  const lines = text.split('\n');
  const i = lines.findIndex((l) => isAttribution(l) || l.startsWith('>'));
  if (i < 0) return [text.trim(), ''];
  return [lines.slice(0, i).join('\n').trim(), lines.slice(i).join('\n')];
}
