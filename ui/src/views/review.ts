// AI Proposals: everything AI agents propose waits here for the user.

import { api } from '../api';
import { h, icon, replace } from '../lib/dom';
import { displayName, formatAddress, fullDate, parseAddresses, relative, timeRange } from '../lib/format';
import { copyBlock, errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { CalEvent, Contact, Draft, Proposal, ProposalAction } from '../types';
import { openComposer, sendWithUndo } from './mail/composer';

// Not 'labels': adding Trash or Junk as a label moves mail there (0040 second pass, #3).
const LOW_RISK = new Set(['archive', 'mark_read', 'flag']);

export class ReviewView {
  root = h('div.review-view');
  private tab: 'pending' | 'history' = 'pending';
  /** Your edits to proposed drafts, by proposal, kept across redraws. */
  private edits = new Map<string, Draft>();
  /** News arrived while you were typing; redraw when you stop. */
  private stale = false;

  constructor() {
    store.onEvent((e) => {
      if ((e.event === 'proposals_changed' || e.event === 'data_changed') && store.state.view === 'review') {
        if (this.typing()) this.stale = true;
        else void this.render();
      }
    });
    this.root.addEventListener('focusout', () => {
      // After the focus has moved on, so typing() sees where it went.
      setTimeout(() => {
        if (this.stale && !this.typing()) {
          this.stale = false;
          void this.render();
        }
      });
    });
  }

  private typing(): boolean {
    const a = document.activeElement;
    return !!a && this.root.contains(a) && (a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement);
  }

  show() {
    void this.render();
  }

  private async render() {
    let items: Proposal[];
    try {
      items = await api.listProposals(null);
    } catch (e) {
      errorToast(e);
      return;
    }
    const pending = items.filter((p) => p.status === 'pending' || p.status === 'stale');
    const history = items.filter((p) => !(p.status === 'pending' || p.status === 'stale'));
    const shown = this.tab === 'pending' ? pending : history;
    const lowRisk = pending.filter((p) => p.status === 'pending' && LOW_RISK.has(p.action.type) && !p.warnings.length);
    const cards = await Promise.all(shown.map((p) => this.card(p)));
    replace(
      this.root,
      h('header.page-head',
        h('h1', 'AI Proposals'),
        h('p.muted', 'AI agents can only propose. Nothing is sent, moved, scheduled or added to your contacts until you approve it here.'),
        h('div.tabs', { role: 'tablist' },
          h(`button.tab${this.tab === 'pending' ? '.active' : ''}`, { role: 'tab', title: 'Proposals waiting for your decision', on: { click: () => { this.tab = 'pending'; void this.render(); } } }, `Waiting (${pending.length})`),
          h(`button.tab${this.tab === 'history' ? '.active' : ''}`, { role: 'tab', title: 'Proposals you approved or rejected', on: { click: () => { this.tab = 'history'; void this.render(); } } }, 'History'),
          h('span.spacer'),
          this.tab === 'pending' && lowRisk.length > 1 &&
            h('button', { title: 'Archive, mark read or flag as proposed, for every waiting proposal without warnings', on: { click: busy(() => this.approveAll(lowRisk)) } }, icon('check', 16), `Approve ${lowRisk.length} low-risk`))),
      shown.length
        ? h('div.proposal-list', cards)
        : h('div.empty-state', icon('sparkle', 36), h('p', this.tab === 'pending' ? 'Nothing waiting for review.' : 'No decisions yet.'),
            this.tab === 'pending' && !items.length && await this.getStarted()),
    );
  }

  /** Until the first proposal arrives: a prompt to paste into any AI tool. */
  private async getStarted(): Promise<HTMLElement> {
    const mi = store.state.settings?.mirror;
    const root = (await api.info().catch(() => null))?.mirror_root ?? '~/Satchel';
    if (mi && (!mi.enabled || !mi.accept_proposals)) {
      return h('p.muted.small', 'To let AI tools propose changes, turn on the AI folder and proposal files in Settings → AI folder. Tools that use MCP can connect in Settings → AI agents.');
    }
    return h('div.get-started',
      h('p.small', 'To get started, paste this into an AI tool that can read and write files on this computer (Claude Code, Codex, Cursor and similar). It needs no setup.'),
      copyBlock(agentPrompt(root)),
      h('p.muted.small', 'Tools that use MCP can connect in Settings → AI agents instead.'));
  }

  private async approveAll(list: Proposal[]) {
    let ok = 0;
    for (const p of list) {
      try {
        await api.approveProposal(p.id);
        ok++;
      } catch (e) {
        errorToast(e);
      }
    }
    toast(`Approved ${ok} of ${list.length} proposal${list.length === 1 ? '' : 's'}`, { quiet: ok === list.length });
    void store.refreshProposals();
    void this.render();
  }

  private async card(p: Proposal): Promise<HTMLElement> {
    const pending = p.status === 'pending' || p.status === 'stale';
    const preview = await this.preview(p.action).catch((e) => h('p.error', `Could not load preview: ${e instanceof Error ? e.message : e}`));
    let editedDraft: Draft | null = null;
    if (p.action.type === 'draft' && pending) {
      // Review shows only the text, so only the text is saved: an agent's
      // body_html never reaches Drafts unseen.
      editedDraft = this.edits.get(p.id) ?? { ...structuredClone(p.action.draft), body_html: null };
      this.edits.set(p.id, editedDraft);
    }
    const body = p.action.type === 'draft' && pending ? this.draftEditor(editedDraft!) : preview;
    const statusChip = h(`span.status-chip.${p.status}`, p.status === 'applied' ? 'approved' : p.status);
    return h(`article.proposal${p.warnings.length ? '.has-warnings' : ''}`,
      h('header.proposal-head',
        h('span.agent-badge', icon('sparkle', 14), p.agent_name || 'Agent'),
        h('h3', p.summary),
        h('span.spacer'),
        !pending || p.status === 'stale' ? statusChip : null,
        h('time.muted.small', { title: fullDate(p.created_at) }, relative(p.created_at))),
      p.status === 'stale' && h('div.banner.warn', icon('warning', 16), 'The message or event changed after this was proposed. Check it again before approving.'),
      p.warnings.length > 0 && h('ul.warnings', p.warnings.map((w) => h('li', icon('warning', 14), w))),
      p.rationale && h('blockquote.rationale', p.rationale),
      body,
      p.review_note && h('p.muted.small', `Note: ${p.review_note}`),
      pending &&
        h('footer.proposal-actions',
          p.action.type === 'draft'
            ? [
                h('button.primary', { title: 'Send this message as shown, after a short undo window', on: { click: busy(() => this.approve(p, editedDraft, 'send')) } }, icon('send', 16), 'Send'),
                h('button', { title: approveHint(p.action), on: { click: busy(() => this.approve(p, editedDraft, 'open')) } }, icon('draft', 16), 'Save as draft'),
              ]
            : h('button.primary', { title: approveHint(p.action), on: { click: busy(() => this.approve(p, editedDraft)) } }, icon('check', 16), 'Approve'),
          h('button', { title: 'Decline this proposal; nothing changes', on: { click: () => void this.reject(p) } }, 'Reject'),
          p.source_thread_ids.length > 0 && h('button.link', { title: 'Open the conversation this proposal is about', on: { click: () => this.openSource(p.source_thread_ids[0]) } }, 'View conversation')),
    );
  }

  private draftEditor(d: Draft): HTMLElement {
    const to = h('input', { type: 'text', value: d.to.map(formatAddress).join(', '), 'aria-label': 'To' });
    const subject = h('input', { type: 'text', value: d.subject, 'aria-label': 'Subject' });
    const body = h('textarea.draft-body', { rows: 10, 'aria-label': 'Message' });
    body.value = d.body_text;
    to.addEventListener('input', () => (d.to = parseAddresses(to.value)));
    subject.addEventListener('input', () => (d.subject = subject.value));
    body.addEventListener('input', () => (d.body_text = body.value));
    const acct = store.state.accounts.find((a) => a.id === d.account_id);
    return h('div.draft-preview',
      h('div.kv', h('span', 'From'), h('span', acct?.email ?? d.account_id)),
      h('label.kv', h('span', 'To'), to),
      d.cc.length > 0 && h('div.kv', h('span', 'Cc'), h('span', d.cc.map(formatAddress).join(', '))),
      d.bcc.length > 0 && h('div.kv', h('span', 'Bcc'), h('span', d.bcc.map(formatAddress).join(', '))),
      h('label.kv', h('span', 'Subject'), subject),
      d.attachments.length > 0 && h('div.kv', h('span', 'Attached'), h('span', d.attachments.map((a) => a.filename).join(', '))),
      body,
      h('p.muted.small', 'Send sends exactly what you see here. Save as draft puts it in Drafts and opens it instead.'));
  }

  private async preview(a: ProposalAction): Promise<HTMLElement> {
    const msgList = async (ids: string[]) => {
      const shown = await Promise.all(ids.slice(0, 5).map((id) => api.getMessage(id).catch(() => null)));
      return h('ul.target-list',
        shown.map((m) => m && h('li', h('strong', displayName(m.from)), ' — ', m.subject || '(no subject)')),
        ids.length > 5 && h('li.muted', `and ${ids.length - 5} more`));
    };
    const mbName = (id: string) => store.mailbox(id)?.path ?? id;
    switch (a.type) {
      case 'draft':
        return h('div.draft-preview', h('div.kv', h('span', 'To'), h('span', a.draft.to.map(formatAddress).join(', '))), h('div.kv', h('span', 'Subject'), h('span', a.draft.subject)), h('pre.draft-text', a.draft.body_text));
      case 'archive':
      case 'mark_read':
      case 'flag':
        return msgList(a.message_ids);
      case 'move':
        return h('div', h('p', 'To ', h('strong', mbName(a.mailbox_id))), await msgList(a.message_ids));
      case 'labels':
        return h('div',
          a.add.length > 0 && h('p', 'Add: ', a.add.map(mbName).join(', ')),
          a.remove.length > 0 && h('p', 'Remove: ', a.remove.map(mbName).join(', ')),
          await msgList(a.message_ids));
      case 'create_event':
        return eventPreview(a.event);
      case 'update_event': {
        const before = a.event.id ? await api.getEvent(a.event.id).catch(() => null) : null;
        return h('div.event-diff', before && h('div.before', h('span.muted.small', 'Now'), eventPreview(before)), h('div.after', h('span.muted.small', 'Proposed'), eventPreview(a.event)));
      }
      case 'cancel_event':
      case 'respond_to_invite': {
        const ev = await api.getEvent(a.event_id).catch(() => null);
        return h('div', a.type === 'respond_to_invite' && h('p', 'Response: ', h('strong', a.response.replace('_', ' '))), ev ? eventPreview(ev) : h('p.muted', 'Event no longer exists.'));
      }
      case 'save_contact': {
        const before = a.contact.id ? await api.getContact(a.contact.id).catch(() => null) : null;
        return before
          ? h('div.event-diff', h('div.before', h('span.muted.small', 'Now'), contactPreview(before)), h('div.after', h('span.muted.small', 'Proposed'), contactPreview(a.contact)))
          : contactPreview(a.contact);
      }
      case 'merge_contacts': {
        const people = await Promise.all(a.contact_ids.map((id) => api.getContact(id).catch(() => null)));
        return h('div',
          people.map((c, i) => h('div',
            h('span.muted.small', i === 0 ? 'Keep' : 'Merge into it'),
            c ? contactPreview(c) : h('p.muted', 'Contact no longer exists.'))));
      }
    }
  }

  /** For a draft, `then` says whether to send the saved draft or open it in the composer. */
  private async approve(p: Proposal, edited: Draft | null, then: 'send' | 'open' = 'open') {
    // Check before approving: a send that fails later would leave the proposal approved but unsent.
    if (then === 'send' && edited && !edited.to.length && !edited.cc.length && !edited.bcc.length) {
      toast('Add at least one recipient to send', { kind: 'error' });
      return;
    }
    try {
      const changed = edited && JSON.stringify(edited) !== JSON.stringify((p.action as { draft?: Draft }).draft);
      const r = await api.approveProposal(p.id, changed ? { type: 'draft', draft: edited! } : null);
      this.edits.delete(p.id);
      if (r.draft_id) {
        const drafts = await api.listDrafts();
        const d = drafts.find((x) => x.id === r.draft_id);
        const opts = { title: `Draft from ${p.agent_name}` };
        // Undo, or a failed send, leaves it in Drafts and opens it to fix.
        if (!d) toast('Approved, but the draft could not be found in Drafts', { kind: 'error' });
        else if (then === 'send') sendWithUndo(d, () => openComposer(d, opts));
        else openComposer(d, opts);
      } else {
        toast('Approved', { quiet: true });
      }
      void store.refreshProposals();
      void this.render();
    } catch (e) {
      errorToast(e);
      void this.render();
    }
  }

  private async reject(p: Proposal) {
    const note = h('textarea', { rows: 3, placeholder: 'Optional: tell the agent why (it can read this)' });
    const m = modal('Reject proposal', h('div', h('p', p.summary), note), {
      footer: h('div.row.end',
        h('button', { title: 'Keep the proposal waiting', on: { click: () => m.close() } }, 'Cancel'),
        h('button.primary.danger', {
          title: 'Reject it; nothing changes, and the agent sees your note',
          on: {
            click: async () => {
              m.close();
              try {
                await api.rejectProposal(p.id, note.value.trim() || null);
                void store.refreshProposals();
                void this.render();
              } catch (e) {
                errorToast(e);
              }
            },
          },
        }, 'Reject')),
    });
  }

  private openSource(threadId: string) {
    store.set({ view: 'mail', selectedThread: threadId });
  }
}

/** Instructions for an AI tool with file access; the folder's README has the details. */
function agentPrompt(root: string): string {
  const sep = root.includes('\\') ? '\\' : '/';
  const at = (p: string) => `${root}${sep}${p.replaceAll('/', sep)}`;
  return `You can help me with my email, calendar and contacts through Satchel Mail. It keeps an up-to-date, plain-text copy of them in this folder:

${root}

Start by reading ${at('README.md')}: it explains the layout and how to propose changes. Read and search the files directly. Editing the mail or calendar files changes nothing; they are a copy.

To act (reply, draft a message, archive, move, label, add or change an event, save a contact), write a proposal file to ${at('proposals/inbox/')} as the README describes, with a short "why:" and "agent: <your name>" in its header. I review every proposal in Satchel before anything happens. Check ${at('proposals/processed/')} to see what became of yours.

Never try to send mail or change my accounts any other way.`;
}

/** A click handler that disables the button's group until `run` finishes, so a double click acts once. */
function busy(run: () => Promise<void>): (e: Event) => void {
  return (e) => {
    const btn = e.currentTarget as HTMLButtonElement;
    const group = [...(btn.parentElement?.querySelectorAll('button') ?? [btn])];
    group.forEach((b) => (b.disabled = true));
    void run().finally(() => group.forEach((b) => (b.disabled = false)));
  };
}

/** What pressing Approve will do, for its tooltip. */
function approveHint(a: ProposalAction): string {
  const n = 'message_ids' in a ? a.message_ids.length : 0;
  const msgs = `${n} message${n === 1 ? '' : 's'}`;
  switch (a.type) {
    case 'draft': return 'Save this to Drafts and open it, without sending';
    case 'archive': return `Archive ${msgs}`;
    case 'move': return `Move ${msgs} to ${store.mailbox(a.mailbox_id)?.path ?? 'the folder shown'}`;
    case 'labels': return `Change labels on ${msgs}`;
    case 'mark_read': return `Mark ${msgs} as ${a.seen ? 'read' : 'unread'}`;
    case 'flag': return `${a.flagged ? 'Flag' : 'Unflag'} ${msgs}`;
    case 'create_event': return 'Add this event to your calendar';
    case 'update_event': return 'Change the event as shown';
    case 'cancel_event': return 'Cancel this event';
    case 'respond_to_invite': return `Reply "${a.response.replace('_', ' ')}" to this invitation`;
    case 'save_contact': return 'Save this contact';
    case 'merge_contacts': return `Merge these ${a.contact_ids.length} contacts into the first`;
  }
}

export function contactPreview(c: Contact): HTMLElement {
  return h('div.event-card',
    h('div.event-title', c.name || c.emails[0]?.email || '(no name)'),
    c.emails.length > 0 && h('div.muted', icon('mail', 14), ` ${c.emails.map((e) => e.email).join(', ')}`),
    c.phones.length > 0 && h('div.muted', ` ${c.phones.map((p) => p.number).join(', ')}`),
    (c.title || c.organization) && h('div.muted.small', [c.title, c.organization].filter(Boolean).join(', ')),
    c.birthday && h('div.muted.small', `Birthday: ${c.birthday}`),
    c.notes && h('p.small', c.notes));
}

export function eventPreview(e: CalEvent): HTMLElement {
  const cal = e.calendar_id;
  return h('div.event-card',
    h('div.event-title', e.title),
    h('div.muted', icon('clock', 14), ` ${new Date(e.start).toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' })} · ${timeRange(e.start, e.end, e.all_day)}${e.timezone ? ` (${e.timezone})` : ''}`),
    e.rrule && h('div.muted.small', `Repeats: ${e.rrule}`),
    e.location && h('div.muted', icon('location', 14), ` ${e.location}`),
    e.attendees.length > 0 && h('div.muted.small', icon('user', 14), ` ${e.attendees.map((a) => a.name || a.email).join(', ')}`),
    e.description && h('p.small', e.description),
    h('span.hidden', cal));
}
