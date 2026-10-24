// Conversation reader: a thread as a chat-style stream, plus the inline reply box.

import { api } from '../api';
import { h, icon, replace } from '../lib/dom';
import { avatarColor, displayName, fullDate, humanSize, initials, shortName, timeRange } from '../lib/format';
import { errorToast, modal, popupMenu, toast } from '../lib/ui';
import { store } from '../state';
import { transport } from '../transport';
import type { Address, Draft, MessageDetail, PartStat, Proposal, ReplyMode, Thread } from '../types';
import { isAttribution, looksDesigned, planFold, splitQuote, stripHtmlQuotes, trimUrl } from '../lib/thread';
import { openComposer, sendWithUndo } from './composer';

/** Consecutive messages from one sender within this window share a name line. */
const GROUP_MS = 15 * 60_000;

type Mine = (a: Address | null | undefined) => boolean;

/** "You" in a thread is the account it belongs to (other local accounts are just people). */
export function mineFor(thread: Thread): Mine {
  const own = store.state.accounts.find((a) => a.id === thread.account_id)?.email.toLowerCase();
  return (a) => !!a && (own ? a.email.toLowerCase() === own : store.state.myEmails.has(a.email.toLowerCase()));
}

const normId = (id: string | null) => (id ?? '').trim().replace(/^<|>$/g, '').toLowerCase();

/** Renders a thread as a stream of bubbles (person-to-person mail) and cards (formatted mail). */
export class Conversation {
  root = h('div.convo');
  private fold: HTMLElement | null = null;
  private newMarker: HTMLElement | null = null;
  private mine: Mine;

  constructor(private thread: Thread, private onReply: (mode: ReplyMode, messageId: string) => void) {
    this.mine = mineFor(thread);
    this.render();
  }

  /** Scroll the reader so the first new message (or the latest one) is in view. */
  settle(scroller: HTMLElement) {
    if (this.newMarker) this.newMarker.scrollIntoView({ block: 'start' });
    else scroller.scrollTop = scroller.scrollHeight;
  }

  private render() {
    const msgs = this.thread.messages;
    const byHeader = new Map(msgs.map((m) => [normId(m.message_id_header), m]));
    const { firstNew, fold } = planFold(msgs, this.mine);

    let lastDay = '';
    let prev: MessageDetail | null = null;
    let grouped: MessageDetail | null = null;
    const foldBox = h('div.convo-fold', { hidden: true });
    for (const [i, m] of msgs.entries()) {
      const out: Node[] = [];
      const day = dayLabel(m.date);
      if (day !== lastDay) {
        out.push(h('div.convo-sys', day));
        lastDay = day;
        grouped = null;
      }
      if (prev) {
        for (const line of recipientChanges(prev, m, this.mine)) out.push(h('div.convo-sys', line));
        if (out.length) grouped = null;
      }
      if (i === firstNew && i > 0) {
        this.newMarker = h('div.convo-new', h('span', 'New'));
        out.push(this.newMarker);
        grouped = null;
      }
      const sameGroup = !!grouped && grouped.from?.email === m.from?.email && Date.parse(m.date) - Date.parse(grouped.date) < GROUP_MS;
      const parent = byHeader.get(normId(m.in_reply_to));
      out.push(this.message(m, !sameGroup, parent && parent !== prev ? parent : null));
      grouped = m;
      prev = m;

      if (fold && i >= fold.start && i < fold.end) foldBox.append(...out);
      else this.root.append(...out);
      if (fold && i === fold.start - 1) {
        this.fold = foldBox;
        const pill = h('button.convo-pill', { on: { click: () => this.unfold() } }, `${fold.end - fold.start} earlier messages`);
        this.root.append(pill, foldBox);
      }
    }
  }

  private unfold() {
    if (!this.fold) return;
    this.fold.hidden = false;
    this.fold.previousElementSibling?.remove();
    this.fold = null;
  }

  private jumpTo(m: MessageDetail) {
    const el = this.root.querySelector<HTMLElement>(`[data-msg="${CSS.escape(m.id)}"]`);
    if (!el) return;
    if (el.closest('.convo-fold')) this.unfold();
    el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    el.classList.add('flash');
    setTimeout(() => el.classList.remove('flash'), 1200);
  }

  private message(m: MessageDetail, showName: boolean, parent: MessageDetail | null): HTMLElement {
    const formatted = m.html_body ? stripHtmlQuotes(m.html_body) : null;
    const asCard = !!formatted && (!m.text_body.trim() || looksDesigned(formatted.html));
    return asCard ? this.card(m, formatted!) : this.bubble(m, showName, parent);
  }

  private menu(anchor: HTMLElement, m: MessageDetail, swap?: () => void) {
    popupMenu(anchor, [
      { label: 'Reply', icon: 'reply', run: () => this.onReply('reply', m.id) },
      { label: 'Reply all', icon: 'replyAll', run: () => this.onReply('reply_all', m.id) },
      { label: 'Forward', icon: 'forward', run: () => this.onReply('forward', m.id) },
      '-',
      ...(swap ? [{ label: 'Show original formatting', icon: 'image', run: swap }] : []),
      { label: 'View source', icon: 'draft', run: () => void viewSource(m.id) },
    ]);
  }

  private bubble(m: MessageDetail, showName: boolean, parent: MessageDetail | null): HTMLElement {
    const mine = this.mine(m.from);
    const body = h(`div.bubble${mine ? '.out' : ''}`, { title: fullDate(m.date) });
    if (parent) {
      body.append(h('button.bubble-quote', { title: 'Jump to this message', on: { click: () => this.jumpTo(parent) } },
        h('strong', `${this.mine(parent.from) ? 'You' : shortName(parent.from)}: `), parent.snippet));
    }
    if (m.invite) body.append(inviteCard(m));
    if (m.body_pending) body.append(h('p.muted', 'Downloading message…'));
    else body.append(textBody(m.text_body));
    const atts = attachments(m);
    if (atts) body.append(atts);

    const row = h(`div.convo-row${mine ? '.mine' : ''}`, { dataset: { msg: m.id } });
    const swap = m.html_body ? () => row.replaceWith(this.card(m, { html: m.html_body!, quoted: null })) : undefined;
    const more = h('button.icon-btn.convo-more', { 'aria-label': 'Message actions', on: { click: (e: MouseEvent) => this.menu(e.currentTarget as HTMLElement, m, swap) } }, icon('more', 16));
    const time = h('time', { datetime: m.date }, clock(m.date));
    if (showName) row.append(h('div.convo-meta', mine ? time : [h('span', displayName(m.from)), time]));
    row.append(h('div.convo-line',
      !mine && (showName ? avatar(m.from) : h('span.avatar-gap')),
      body,
      more));
    return row;
  }

  private card(m: MessageDetail, formatted: { html: string; quoted: string | null }): HTMLElement {
    const card = h('article.convo-card', { dataset: { msg: m.id } });
    const head = h('header.convo-card-head',
      avatar(m.from),
      h('div.grow', h('strong', displayName(m.from)), m.from && h('span.muted.small', ` ${m.from.email}`)),
      h('time.muted.small', { datetime: m.date, title: fullDate(m.date) }, clock(m.date)),
      h('button.icon-btn', { 'aria-label': 'Message actions', on: { click: (e: MouseEvent) => this.menu(e.currentTarget as HTMLElement, m) } }, icon('more', 16)));
    card.append(head);
    if (m.remote_images_blocked) {
      card.append(
        h('div.banner',
          icon('image', 16),
          h('span', 'Remote images are hidden to protect your privacy.'),
          h('button.link', { on: { click: () => void this.loadImages(m, false, card) } }, 'Show once'),
          h('button.link', { on: { click: () => void this.loadImages(m, true, card) } }, `Always for ${m.from?.email ?? 'sender'}`)),
      );
    }
    if (m.invite) card.append(inviteCard(m));
    const frameHost = h('div', htmlFrame(formatted.html));
    card.append(frameHost);
    if (formatted.quoted) {
      const full = formatted.quoted;
      card.append(h('button.quote-toggle', {
        title: 'Show quoted text',
        'aria-label': 'Show quoted text',
        on: { click: (e: MouseEvent) => { replace(frameHost, htmlFrame(full)); (e.currentTarget as HTMLElement).remove(); } },
      }, '•••'));
    }
    const atts = attachments(m);
    if (atts) card.append(atts);
    return card;
  }

  private async loadImages(m: MessageDetail, always: boolean, card: HTMLElement) {
    try {
      const fresh = await api.loadRemoteImages(m.id, always);
      const i = this.thread.messages.findIndex((x) => x.id === m.id);
      if (i >= 0) this.thread.messages[i] = fresh;
      card.replaceWith(this.card(fresh, stripHtmlQuotes(fresh.html_body ?? '')));
    } catch (e) {
      errorToast(e);
    }
  }
}

function avatar(a: Address | null | undefined) {
  return h('span.avatar', { style: { background: avatarColor(a?.email ?? '') }, 'aria-hidden': 'true' }, initials(a));
}

function attachments(m: MessageDetail): HTMLElement | null {
  const atts = m.attachments.filter((a) => !a.inline);
  if (!atts.length) return null;
  return h('div.attachments',
    atts.map((a) =>
      h('button.attachment', { title: `Download ${a.filename}`, on: { click: () => void download(m.id, a.id) } },
        icon('paperclip', 16), h('span.name', a.filename), h('span.muted.small', humanSize(a.size)))));
}

/** "Bob added Dan" / "Carol removed you": who joined or left between two messages. */
function recipientChanges(prev: MessageDetail, m: MessageDetail, isMine: Mine): string[] {
  const people = (x: MessageDetail) => {
    const map = new Map<string, Address>();
    for (const a of [x.from, ...x.to, ...x.cc]) if (a) map.set(a.email.toLowerCase(), a);
    return map;
  };
  const before = people(prev);
  const after = people(m);
  const names = (list: Address[]) => list.map((a) => (isMine(a) ? 'you' : shortName(a))).join(', ');
  const added = [...after].filter(([k]) => !before.has(k)).map(([, a]) => a);
  const removed = [...before].filter(([k]) => !after.has(k)).map(([, a]) => a);
  const who = isMine(m.from) ? 'You' : shortName(m.from);
  const out: string[] = [];
  if (added.length) out.push(`${who} added ${names(added)}`);
  if (removed.length) out.push(`${who} removed ${names(removed)}`);
  return out;
}

function clock(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

function dayLabel(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const day = new Date(d);
  day.setHours(0, 0, 0, 0);
  const diff = Math.round((today.getTime() - day.getTime()) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  if (diff > 1 && diff < 7) return d.toLocaleDateString([], { weekday: 'long' });
  return d.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric', year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric' });
}

// ------------------------------------------------------------ inline reply

/** Unsent inline replies by thread, so switching threads doesn't lose them. */
const pendingText = new Map<string, string>();

/** Always-open reply box under a conversation; shows an agent's pending draft when there is one. */
export class ReplyBox {
  root = h('div.reply-box');
  private input = h('textarea.reply-input', { rows: 1, placeholder: 'Write a reply…', 'aria-label': 'Reply' });
  private to = h('button.reply-to');
  private note = h('span.reply-note');
  private base: Draft | null = null;
  private proposal: Proposal | null = null;
  private mode: ReplyMode = 'reply_all';
  private target: string;
  private mine: Mine;

  constructor(private thread: Thread) {
    const msgs = thread.messages;
    this.mine = mineFor(thread);
    this.target = [...msgs].reverse().find((m) => !this.mine(m.from))?.id ?? msgs[msgs.length - 1].id;
    this.input.value = pendingText.get(thread.thread_id) ?? '';
    this.input.addEventListener('input', () => {
      pendingText.set(thread.thread_id, this.input.value);
      this.grow();
    });
    this.input.addEventListener('keydown', (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        void this.send();
      } else if (e.key === 'Escape') {
        // Leave the box without also closing the conversation.
        e.stopPropagation();
        this.input.blur();
      }
    });
    this.to.addEventListener('click', () => {
      if (this.proposal) return;
      void this.load(this.mode === 'reply_all' ? 'reply' : 'reply_all');
    });
    this.root.append(
      h('div.reply-head', this.to, this.note),
      this.input,
      h('div.reply-foot',
        h('button.icon-btn', { title: 'Open in full composer', 'aria-label': 'Open in full composer', on: { click: () => this.expand() } }, icon('compose', 16)),
        h('span.spacer'),
        h('button.discard', { hidden: true, on: { click: () => void this.discard() } }, 'Discard'),
        h('button.primary', { title: 'Send (⌘↩)', on: { click: () => void this.send() } }, 'Send')),
    );
    void this.init();
  }

  focus(mode?: ReplyMode) {
    if (mode && mode !== this.mode && !this.proposal) void this.load(mode);
    this.input.focus();
  }

  private grow() {
    this.input.style.height = 'auto';
    this.input.style.height = `${Math.min(this.input.scrollHeight, window.innerHeight * 0.4)}px`;
  }

  private async init() {
    try {
      const pending = await api.listProposals('pending');
      const p = pending
        .filter((x) => x.action.type === 'draft' && !x.action.draft.forward_of && x.source_thread_ids.includes(this.thread.thread_id))
        .sort((a, b) => b.created_at.localeCompare(a.created_at))[0];
      if (p && p.action.type === 'draft') {
        this.proposal = p;
        this.base = p.action.draft;
        if (!this.input.value) this.input.value = splitQuote(p.action.draft.body_text)[0];
        this.note.textContent = `Draft from ${p.agent_name} — edit or send`;
        this.root.classList.add('proposed');
        this.root.querySelector<HTMLElement>('.discard')!.hidden = false;
        this.showRecipients();
        this.grow();
        return;
      }
    } catch {
      /* proposals are optional here; fall back to a plain reply */
    }
    const others = new Set(this.thread.messages.flatMap((m) => [m.from, ...m.to, ...m.cc]).filter((a) => a && !this.mine(a)).map((a) => a!.email.toLowerCase()));
    await this.load(others.size > 1 ? 'reply_all' : 'reply');
    this.grow();
  }

  private async load(mode: ReplyMode) {
    try {
      this.base = await api.composeReply(this.target, mode);
      this.mode = mode;
      this.showRecipients();
    } catch (e) {
      errorToast(e);
    }
  }

  private showRecipients() {
    const all = [...(this.base?.to ?? []), ...(this.base?.cc ?? [])];
    const label = this.proposal ? 'To' : this.mode === 'reply_all' ? 'Reply all' : 'Reply';
    this.to.textContent = `${label} · ${all.map(shortName).join(', ') || 'no recipients'}`;
    this.to.title = this.proposal ? all.map(displayName).join(', ') : 'Switch between reply and reply all';
  }

  private draft(): Draft | null {
    if (!this.base) return null;
    const typed = this.input.value.trimEnd();
    const quoted = splitQuote(this.base.body_text)[1];
    return { ...this.base, body_text: quoted ? `${typed}\n\n${quoted.trimStart()}` : typed };
  }

  private clear() {
    this.input.value = '';
    pendingText.delete(this.thread.thread_id);
    this.grow();
  }

  private async send() {
    const d = this.draft();
    if (!d) return;
    if (!this.input.value.trim()) {
      toast('Write something first.', { kind: 'error' });
      this.input.focus();
      return;
    }
    try {
      if (this.proposal) {
        const r = await api.approveProposal(this.proposal.id, { type: 'draft', draft: d });
        d.id = r.draft_id ?? null;
        this.proposal = null;
        this.root.classList.remove('proposed');
        this.note.textContent = '';
        this.root.querySelector<HTMLElement>('.discard')!.hidden = true;
      }
      this.clear();
      sendWithUndo(d, () => openComposer(d));
    } catch (e) {
      errorToast(e);
    }
  }

  private async discard() {
    if (!this.proposal) return;
    try {
      await api.rejectProposal(this.proposal.id, null);
      this.proposal = null;
      this.root.classList.remove('proposed');
      this.note.textContent = '';
      this.root.querySelector<HTMLElement>('.discard')!.hidden = true;
      this.clear();
      await this.load(this.mode);
    } catch (e) {
      errorToast(e);
    }
  }

  private expand() {
    const d = this.draft();
    if (!d) return;
    this.clear();
    openComposer(d);
  }
}

// ------------------------------------------------------------ message bodies

function inviteCard(m: MessageDetail): HTMLElement {
  const inv = m.invite!;
  const when = `${new Date(inv.start).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })} · ${timeRange(inv.start, inv.end, inv.all_day)}`;
  if (inv.method === 'CANCEL') {
    return h('div.invite.cancelled', icon('calendar', 20), h('div', h('strong', `Cancelled: ${inv.title}`), h('div.muted.small', when)));
  }
  if (inv.method !== 'REQUEST') {
    return h('div.invite', icon('calendar', 20), h('div', h('strong', inv.title), h('div.muted.small', when)));
  }
  const card = h('div.invite');
  const render = (status: PartStat | null) => {
    replace(card,
      icon('calendar', 22),
      h('div.grow',
        h('strong', inv.title),
        h('div.muted.small', when),
        inv.location && h('div.muted.small', icon('location', 12), ` ${inv.location}`),
        inv.organizer && h('div.muted.small', `From ${inv.organizer.name ?? inv.organizer.email}`)),
      h('div.rsvp-buttons',
        (['accepted', 'tentative', 'declined'] as PartStat[]).map((r) =>
          h(`button${status === r ? '.active' : ''}`, {
            on: {
              click: async () => {
                try {
                  const res = await api.respondToEmailInvite(m.id, r);
                  render(r);
                  toast(res.via === 'calendar' ? 'Response saved to your calendar' : 'Reply sent to the organizer');
                } catch (e) {
                  errorToast(e);
                }
              },
            },
          }, r === 'accepted' ? 'Yes' : r === 'declined' ? 'No' : 'Maybe'))));
  };
  render(inv.my_status);
  return card;
}

function linkified(line: string): HTMLElement {
  const div = h(line.startsWith('>') ? 'div.quote' : 'div');
  for (const p of line.split(/(https?:\/\/[^\s<>"]+)/g)) {
    if (/^https?:\/\//.test(p)) {
      const [url, rest] = trimUrl(p);
      div.appendChild(h('a', { href: url, rel: 'noopener noreferrer', target: '_blank', on: { click: (e: MouseEvent) => { e.preventDefault(); openExternal(url); } } }, url));
      if (rest) div.appendChild(document.createTextNode(rest));
    } else if (p) div.appendChild(document.createTextNode(p));
  }
  if (!line) div.appendChild(h('br'));
  return div;
}

/** Plain text body: linkified, with quoted history folded behind a toggle. */
function textBody(text: string): HTMLElement {
  const pre = h('div.text-body');
  const lines = text.replace(/\r\n?/g, '\n').replace(/\s+$/, '').split('\n');
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (isAttribution(line) || line.startsWith('>')) {
      const start = i;
      while (i < lines.length && (lines[i].startsWith('>') || isAttribution(lines[i]) || (lines[i].trim() === '' && lines[i + 1]?.startsWith('>')))) i++;
      // Fold real history or a trailing quote, not a single quoted line mid-reply.
      if (i - start >= 2 || isAttribution(lines[start]) || lines.slice(i).every((l) => !l.trim())) {
        while (pre.lastElementChild?.matches('div') && !pre.lastElementChild.textContent?.trim()) pre.lastElementChild.remove();
        const block = h('div.quoted', { hidden: true }, lines.slice(start, i).map(linkified));
        pre.append(h('button.quote-toggle', { title: 'Show quoted text', 'aria-label': 'Show quoted text', on: { click: (e: MouseEvent) => { block.hidden = !block.hidden; (e.currentTarget as HTMLElement).classList.toggle('open'); } } }, '•••'), block);
        continue;
      }
      i = start;
    }
    pre.appendChild(linkified(line));
    i++;
  }
  return pre;
}

/** Sanitized HTML (from the core) in a script-less sandboxed iframe with a CSP. */
function htmlFrame(html: string): HTMLElement {
  const csp = "default-src 'none'; img-src data: https: http:; style-src 'unsafe-inline'; font-src data:";
  const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><base target="_blank"><style>
    html,body{margin:0;padding:0;background:transparent;color:#1d1d1f;font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;overflow-wrap:anywhere}
    img{max-width:100%;height:auto} table{max-width:100%} blockquote{border-left:3px solid #ccc;margin:0 0 0 .5em;padding-left:.75em;color:#555}
    a{color:#2a62d9}
  </style></head><body>${html}</body></html>`;
  const frame = h('iframe.html-body', { sandbox: 'allow-same-origin allow-popups allow-popups-to-escape-sandbox', referrerpolicy: 'no-referrer', title: 'Message content' }) as HTMLIFrameElement;
  frame.srcdoc = doc;
  frame.addEventListener('load', () => {
    const d = frame.contentDocument;
    if (!d) return;
    const fit = () => (frame.style.height = `${Math.min(d.documentElement.scrollHeight + 4, 20000)}px`);
    fit();
    new ResizeObserver(fit).observe(d.body);
    d.addEventListener('click', (e) => {
      const a = (e.target as HTMLElement).closest('a');
      if (a?.href && /^(https?|mailto):/.test(a.href)) {
        e.preventDefault();
        if (a.href.startsWith('mailto:')) {
          const email = decodeURIComponent(a.href.slice(7).split('?')[0]);
          openComposer({ to: [{ email }] });
        } else openExternal(a.href);
      }
    });
  });
  return h('div.html-wrap', frame);
}

export function openExternal(url: string) {
  if (transport.kind === 'tauri') void transport.invoke('open_url', { url }).catch(() => window.open(url, '_blank', 'noopener'));
  else window.open(url, '_blank', 'noopener,noreferrer');
}

async function download(messageId: string, attachmentId: string) {
  try {
    const a = await api.getAttachment(messageId, attachmentId);
    await saveBase64(a.filename, a.mime_type, a.data_base64);
  } catch (e) {
    errorToast(e);
  }
}

export async function saveBase64(filename: string, mime: string, b64: string) {
  if (transport.kind === 'tauri') {
    const path = await transport.invoke<string | null>('save_file', { filename, data_base64: b64 });
    if (path) toast(`Saved to ${path}`, { kind: 'success' });
    return;
  }
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const url = URL.createObjectURL(new Blob([bytes], { type: mime }));
  const link = h('a', { href: url, download: filename });
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

export async function viewSource(messageId: string) {
  try {
    const { raw } = await api.getRaw(messageId);
    modal('Message source', h('pre.source', raw), { wide: true });
  } catch (e) {
    errorToast(e);
  }
}
