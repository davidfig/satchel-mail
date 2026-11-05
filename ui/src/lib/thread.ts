// Pure thread helpers: quote detection and which messages to fold. No app imports,
// so they run under `node --test` (see ui/test/).

import type { Address, MessageDetail } from '../types';

/** Show the first message, a fold, then this many before the first new one. */
const CONTEXT_BEFORE_NEW = 1;
/** Always show at least this many of the latest messages. */
const MIN_RECENT = 3;
/** Folding fewer messages than this isn't worth a click. */
const MIN_FOLD = 2;

/**
 * Splits trailing punctuation off a URL found in text, so "(https://x.test/a)." links
 * to https://x.test/a. Brackets the URL itself opened stay (Wikipedia-style links).
 */
export function trimUrl(url: string): [string, string] {
  const count = (s: string, c: string) => s.split(c).length - 1;
  let end = url.length;
  while (end > 0) {
    const c = url[end - 1];
    const body = url.slice(0, end);
    if ('.,;:!?\'"'.includes(c)) end--;
    else if (c === ')' && count(body, '(') < count(body, ')')) end--;
    else if (c === ']' && count(body, '[') < count(body, ']')) end--;
    else break;
  }
  return [url.slice(0, end), url.slice(end)];
}

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

/** Which kind of message link we follow: web pages and email addresses, nothing else. */
export function linkScheme(url: string): 'web' | 'mail' | null {
  if (/^https?:/i.test(url)) return 'web';
  if (/^mailto:/i.test(url)) return 'mail';
  return null;
}

/** Our own page that shows a message link and asks before opening it (web mode). */
export function followHref(origin: string, url: string, cautious: boolean): string {
  return `${origin}/#follow=${encodeURIComponent(url)}${cautious ? '&cautious=1' : ''}`;
}

/** The link a `followHref` address carries, or null if `href` isn't one for `origin`. */
export function followTarget(href: string, origin: string): { url: string; cautious: boolean } | null {
  let u: URL;
  try {
    u = new URL(href);
  } catch {
    return null;
  }
  if (u.origin !== origin || u.pathname !== '/') return null;
  const hash = new URLSearchParams(u.hash.slice(1));
  const url = hash.get('follow');
  return url ? { url, cautious: hash.get('cautious') === '1' } : null;
}

/**
 * Web mode: points every link in (already sanitized) HTML at our own `#follow=` page,
 * so a click that no listener catches (Safari runs none in the sandboxed frame) still
 * lands on a page that shows where the link goes and asks. Anchors within the message
 * and `cid:` parts stay as they are.
 */
export function routeLinks(html: string, origin: string, cautious: boolean): string {
  const doc = new DOMParser().parseFromString(`<!doctype html><html><head></head><body>${html}</body></html>`, 'text/html');
  let changed = false;
  for (const a of doc.body.querySelectorAll('a[href], area[href]')) {
    const href = (a.getAttribute('href') ?? '').trim();
    if (!href || href.startsWith('#') || /^cid:/i.test(href)) continue;
    a.setAttribute('href', followHref(origin, href, cautious));
    a.setAttribute('target', '_blank');
    a.setAttribute('rel', 'noopener noreferrer');
    changed = true;
  }
  return changed ? doc.body.innerHTML : html;
}

/**
 * A conversation as Markdown, for pasting into an AI chat. Each body is fenced
 * so the sender's text can't pass itself off as another message. With
 * `trimQuotes`, a reply loses the quoted history the earlier messages already show.
 */
export function threadMarkdown(subject: string, msgs: MessageDetail[], trimQuotes = msgs.length > 1): string {
  const addrs = (list: Address[]) => list.map((a) => (a.name?.trim() ? `${a.name.trim()} <${a.email}>` : a.email)).join(', ');
  const out = [`# ${subject.trim() || '(no subject)'}`, ''];
  for (const m of msgs) {
    out.push(`## ${m.from ? addrs([m.from]) : '(unknown sender)'}`, '');
    out.push(`- date: ${new Date(m.date).toString()}`);
    if (m.to.length) out.push(`- to: ${addrs(m.to)}`);
    if (m.cc.length) out.push(`- cc: ${addrs(m.cc)}`);
    if (m.subject.trim() && m.subject !== subject) out.push(`- subject: ${m.subject.trim()}`);
    const atts = m.attachments.filter((a) => !a.inline).map((a) => a.filename);
    if (atts.length) out.push(`- attachments: ${atts.join(', ')}`);
    if (m.invite) out.push(`- calendar invitation: ${m.invite.title}`);
    if (m.flags.draft) out.push('- draft, not sent');
    if (m.body_pending) out.push('- body not downloaded yet; this is only a preview');
    let body = (m.body_pending ? m.snippet : m.text_body).trim();
    if (trimQuotes) body = splitQuote(body)[0] || body;
    out.push('', fenced(body), '');
  }
  return out.join('\n');
}

/** `s` in a code fence longer than any run of backticks inside it. */
function fenced(s: string): string {
  const longest = Math.max(0, ...(s.match(/`+/g) ?? []).map((r) => r.length));
  const fence = '`'.repeat(Math.max(3, longest + 1));
  return `${fence}text\n${s}\n${fence}`;
}
