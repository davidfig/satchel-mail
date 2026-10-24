// Review queue: everything AI agents propose waits here for the user.

import { api } from '../api';
import { h, icon, replace } from '../lib/dom';
import { displayName, formatAddress, fullDate, relative, timeRange } from '../lib/format';
import { errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { CalEvent, Draft, Proposal, ProposalAction } from '../types';
import { openComposer } from './composer';

const LOW_RISK = new Set(['archive', 'mark_read', 'flag', 'labels']);

export class ReviewView {
  root = h('div.review-view');
  private tab: 'pending' | 'history' = 'pending';

  constructor() {
    store.onEvent((e) => {
      if ((e.event === 'proposals_changed' || e.event === 'data_changed') && store.state.view === 'review') void this.render();
    });
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
        h('h1', 'Review'),
        h('p.muted', 'AI agents can only propose. Nothing is sent, moved or scheduled until you approve it here.'),
        h('div.tabs', { role: 'tablist' },
          h(`button.tab${this.tab === 'pending' ? '.active' : ''}`, { role: 'tab', on: { click: () => { this.tab = 'pending'; void this.render(); } } }, `Waiting (${pending.length})`),
          h(`button.tab${this.tab === 'history' ? '.active' : ''}`, { role: 'tab', on: { click: () => { this.tab = 'history'; void this.render(); } } }, 'History'),
          h('span.spacer'),
          this.tab === 'pending' && lowRisk.length > 1 &&
            h('button', { title: 'Approve every waiting proposal marked low risk', on: { click: () => void this.approveAll(lowRisk) } }, icon('check', 16), `Approve ${lowRisk.length} low-risk`))),
      shown.length
        ? h('div.proposal-list', cards)
        : h('div.empty-state', icon('sparkle', 36), h('p', this.tab === 'pending' ? 'Nothing waiting for review.' : 'No decisions yet.'),
            this.tab === 'pending' && h('p.muted.small', 'Connect an AI agent in Settings → AI agents to start receiving proposals.')),
    );
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
    toast(`Approved ${ok} proposal${ok === 1 ? '' : 's'}`, { kind: 'success' });
    void this.render();
  }

  private async card(p: Proposal): Promise<HTMLElement> {
    const pending = p.status === 'pending' || p.status === 'stale';
    const preview = await this.preview(p.action).catch((e) => h('p.error', `Could not load preview: ${e instanceof Error ? e.message : e}`));
    let editedDraft: Draft | null = null;
    if (p.action.type === 'draft') editedDraft = structuredClone(p.action.draft);
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
          h('button.primary', { on: { click: () => void this.approve(p, editedDraft) } }, icon('check', 16), p.action.type === 'draft' ? 'Approve & open draft' : 'Approve'),
          h('button', { title: 'Decline this proposal; nothing changes', on: { click: () => void this.reject(p) } }, 'Reject'),
          p.source_thread_ids.length > 0 && h('button.link', { title: 'Open the conversation this proposal is about', on: { click: () => this.openSource(p.source_thread_ids[0]) } }, 'View conversation')),
    );
  }

  private draftEditor(d: Draft): HTMLElement {
    const to = h('input', { type: 'text', value: d.to.map(formatAddress).join(', '), 'aria-label': 'To' });
    const subject = h('input', { type: 'text', value: d.subject, 'aria-label': 'Subject' });
    const body = h('textarea.draft-body', { rows: 10, 'aria-label': 'Message' });
    body.value = d.body_text;
    to.addEventListener('input', () => (d.to = to.value.split(',').map((s) => s.trim()).filter(Boolean).map((e) => { const m = e.match(/^(.*)<([^>]+)>$/); return m ? { name: m[1].trim() || null, email: m[2].trim() } : { email: e }; })));
    subject.addEventListener('input', () => (d.subject = subject.value));
    body.addEventListener('input', () => (d.body_text = body.value));
    const acct = store.state.accounts.find((a) => a.id === d.account_id);
    return h('div.draft-preview',
      h('div.kv', h('span', 'From'), h('span', acct?.email ?? d.account_id)),
      h('label.kv', h('span', 'To'), to),
      d.cc.length > 0 && h('div.kv', h('span', 'Cc'), h('span', d.cc.map(formatAddress).join(', '))),
      d.bcc.length > 0 && h('div.kv', h('span', 'Bcc'), h('span', d.bcc.map(formatAddress).join(', '))),
      h('label.kv', h('span', 'Subject'), subject),
      body,
      h('p.muted.small', 'Approving saves this as a draft and opens it. You still press Send yourself.'));
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
    }
  }

  private async approve(p: Proposal, edited: Draft | null) {
    try {
      const changed = edited && JSON.stringify(edited) !== JSON.stringify((p.action as { draft?: Draft }).draft);
      const r = await api.approveProposal(p.id, changed ? { type: 'draft', draft: edited! } : null);
      if (r.draft_id) {
        const drafts = await api.listDrafts();
        const d = drafts.find((x) => x.id === r.draft_id);
        if (d) openComposer(d, { title: `Draft from ${p.agent_name}` });
      } else {
        toast('Approved', { kind: 'success' });
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
        h('button', { on: { click: () => m.close() } }, 'Cancel'),
        h('button.primary.danger', {
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
