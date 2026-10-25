// Conversation reader: a thread as a chat-style stream.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { fullDate, shortName } from '../../lib/format';
import { avatarLink, openPerson, personLink } from '../../lib/person';
import { contextMenu, errorToast, popupMenu } from '../../lib/ui';
import type { MenuItem } from '../../lib/ui';
import { store } from '../../state';
import type { Address, MessageDetail, ReplyMode, Thread } from '../../types';
import { looksDesigned, planFold, stripHtmlQuotes } from '../../lib/thread';
import { attachments, draftBanner, htmlFrame, inviteCard, textBody, viewSource, warnings } from './message-body';

/** Consecutive messages from one sender within this window share a name line. */
const GROUP_MS = 15 * 60_000;

export type Mine = (a: Address | null | undefined) => boolean;

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

  /** `spam`: the thread is in Spam, so links ask first and invitations can't be answered. */
  constructor(private thread: Thread, private onReply: (mode: ReplyMode, messageId: string) => void, private opts: { spam?: boolean } = {}) {
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
        const pill = h('button.convo-pill', { title: 'Show the earlier messages', on: { click: () => this.unfold() } }, `${fold.end - fold.start} earlier messages`);
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
    popupMenu(anchor, this.menuItems(m, swap));
  }

  private menuItems(m: MessageDetail, swap?: () => void): (MenuItem | '-')[] {
    return [
      { label: 'Reply', icon: 'reply', run: () => this.onReply('reply', m.id) },
      { label: 'Reply all', icon: 'replyAll', run: () => this.onReply('reply_all', m.id) },
      { label: 'Forward', icon: 'forward', run: () => this.onReply('forward', m.id) },
      '-',
      ...(swap ? [{ label: 'Show original formatting', icon: 'image', run: swap }] : []),
      { label: 'View source', icon: 'draft', run: () => void viewSource(m.id) },
      ...(m.from && !this.mine(m.from) ? ['-' as const, { label: `All mail with ${shortName(m.from)}`, icon: 'user', run: () => openPerson(m.from!) }] : []),
    ];
  }

  private bubble(m: MessageDetail, showName: boolean, parent: MessageDetail | null): HTMLElement {
    const mine = this.mine(m.from);
    const body = h(`div.bubble${mine ? '.out' : ''}`, { title: fullDate(m.date) });
    if (parent) {
      body.append(h('button.bubble-quote', { title: 'Jump to this message', on: { click: () => this.jumpTo(parent) } },
        h('strong', `${this.mine(parent.from) ? 'You' : shortName(parent.from)}: `), parent.snippet));
    }
    const warned = warnings(m);
    if (warned) body.append(warned);
    if (m.flags.draft) body.append(draftBanner(m));
    if (m.invite) body.append(inviteCard(m, !!this.opts.spam));
    if (m.body_pending) body.append(h('p.muted', 'Downloading message…'));
    else body.append(textBody(m.text_body, this.cautious(m)));
    const atts = attachments(m);
    if (atts) body.append(atts);

    const row = h(`div.convo-row${mine ? '.mine' : ''}`, { dataset: { msg: m.id } });
    const swap = m.html_body ? () => row.replaceWith(this.card(m, { html: m.html_body!, quoted: null })) : undefined;
    const more = h('button.icon-btn.convo-more', { 'aria-label': 'Message actions', on: { click: (e: MouseEvent) => this.menu(e.currentTarget as HTMLElement, m, swap) } }, icon('more', 16));
    contextMenu(row, () => this.menuItems(m, swap));
    const time = h('time', { datetime: m.date }, clock(m.date));
    if (showName) row.append(h('div.convo-meta', mine ? time : [personLink(m.from), time]));
    row.append(h('div.convo-line',
      !mine && (showName ? avatarLink(m.from) : h('span.avatar-gap')),
      body,
      more));
    return row;
  }

  private card(m: MessageDetail, formatted: { html: string; quoted: string | null }): HTMLElement {
    const card = h('article.convo-card', { dataset: { msg: m.id } });
    const head = h('header.convo-card-head',
      avatarLink(m.from),
      h('div.grow', h('strong', personLink(m.from)), m.from && h('span.muted.small', ` ${m.from.email}`)),
      h('time.muted.small', { datetime: m.date, title: fullDate(m.date) }, clock(m.date)),
      h('button.icon-btn', { 'aria-label': 'Message actions', on: { click: (e: MouseEvent) => this.menu(e.currentTarget as HTMLElement, m) } }, icon('more', 16)));
    card.append(head);
    contextMenu(head, () => this.menuItems(m));
    if (m.flags.draft) card.append(draftBanner(m));
    if (m.remote_images_blocked) {
      card.append(
        h('div.banner',
          icon('image', 16),
          h('span', 'Remote images are hidden to protect your privacy.'),
          h('button.link', { title: 'Load remote images for this message only', on: { click: () => void this.loadImages(m, false, card) } }, 'Show once'),
          h('button.link', { title: 'Always load remote images from this sender', on: { click: () => void this.loadImages(m, true, card) } }, `Always for ${m.from?.email ?? 'sender'}`)),
      );
    }
    const warned = warnings(m);
    if (warned) card.append(warned);
    if (m.invite) card.append(inviteCard(m, !!this.opts.spam));
    const cautious = this.cautious(m);
    const frameHost = h('div', htmlFrame(formatted.html, cautious, m.remote_images_allowed));
    card.append(frameHost);
    if (formatted.quoted) {
      const full = formatted.quoted;
      card.append(h('button.quote-toggle', {
        title: 'Show quoted text',
        'aria-label': 'Show quoted text',
        on: { click: (e: MouseEvent) => { replace(frameHost, htmlFrame(full, cautious, m.remote_images_allowed)); (e.currentTarget as HTMLElement).remove(); } },
      }, '•••'));
    }
    const atts = attachments(m);
    if (atts) card.append(atts);
    return card;
  }

  /** Links in spam or in messages with warnings ask before opening. */
  private cautious(m: MessageDetail): boolean {
    return !!this.opts.spam || !!m.warnings?.length;
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
