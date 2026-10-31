// The inline reply box under a conversation.

import { api } from '../../api';
import { h, icon } from '../../lib/dom';
import { displayName, shortName } from '../../lib/format';
import { errorToast, toast } from '../../lib/ui';
import type { Draft, Proposal, ReplyMode, Thread } from '../../types';
import { splitQuote } from '../../lib/thread';
import { openComposer, sendWithUndo } from './composer';
import { type Mine, mineFor } from './conversation';
import { warnings } from './message-body';

/** Unsent inline replies by thread, so switching threads doesn't lose them. */
const pendingText = new Map<string, string>();

/** Mail nobody expects an answer to, in a thread you've never written in: the
 *  core marks the latest message from someone else as list or machine mail. */
function isBulk(thread: Thread, mine: Mine): boolean {
  const theirs = thread.messages.filter((m) => !mine(m.from));
  const last = theirs[theirs.length - 1];
  return !!last && theirs.length === thread.messages.length && !!last.bulk;
}

/** Reply box under a conversation, open unless it's list or no-reply mail; shows an agent's pending draft when there is one. */
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
      h('button.reply-reopen', { title: 'Reply (r)', on: { click: () => this.focus() } }, icon('reply', 14), 'Reply'),
      h('div.reply-head', this.to, this.note,
        h('span.spacer'),
        h('button.icon-btn', { title: 'Hide reply box (r to reopen)', 'aria-label': 'Hide reply box', on: { click: () => this.dismiss() } }, icon('close', 14))),
      warnings(msgs.find((m) => m.id === this.target)!, true) ?? '',
      this.input,
      h('div.reply-foot',
        h('button.icon-btn', { title: 'Open in full composer', 'aria-label': 'Open in full composer', on: { click: () => this.expand() } }, icon('compose', 16)),
        h('span.spacer'),
        h('button.discard', { hidden: true, title: 'Throw away this reply draft', on: { click: () => void this.discard() } }, 'Discard'),
        h('button.primary', { title: 'Send (⌘↩)', on: { click: () => void this.send() } }, 'Send')),
    );
    // List and no-reply mail rarely gets a reply, so start it as the "Reply" button unless something's already typed.
    if (isBulk(thread, this.mine) && !this.input.value) this.root.classList.add('dismissed');
    void this.init();
  }

  focus(mode?: ReplyMode) {
    if (mode && mode !== this.mode && !this.proposal) void this.load(mode);
    this.root.classList.remove('dismissed');
    this.grow();
    this.input.focus();
  }

  /** Shrinks the box to a "Reply" button to free up reading space; typed text is kept. */
  private dismiss() {
    this.root.classList.add('dismissed');
    this.input.blur();
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
        this.root.classList.remove('dismissed');
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
    this.to.dataset.tooltip = this.proposal ? all.map(displayName).join(', ') : 'Switch between reply and reply all';
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
