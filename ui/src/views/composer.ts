// Message composer (new, reply, forward, AI-proposed drafts).

import { api } from '../api';
import { h, icon } from '../lib/dom';
import { formatAddress, humanSize, parseAddresses } from '../lib/format';
import { confirmDialog, errorToast, toast } from '../lib/ui';
import { store } from '../state';
import type { Draft, OutgoingAttachment } from '../types';

const UNDO_SECONDS = 5;

let open: { close: () => void } | null = null;

function addrField(label: string, value: string, onInput: (v: string) => void) {
  const input = h('input', { type: 'text', value, autocomplete: 'off', spellcheck: false, 'aria-label': label });
  const list = h('div.suggest');
  let timer: number | undefined;
  input.addEventListener('input', () => {
    onInput(input.value);
    clearTimeout(timer);
    const token = input.value.split(',').pop()?.trim() ?? '';
    if (token.length < 2) {
      list.replaceChildren();
      return;
    }
    timer = window.setTimeout(async () => {
      try {
        const res = await api.suggestAddresses(token);
        list.replaceChildren(
          ...res.map((a) =>
            h('button.suggest-item', {
              type: 'button',
              on: {
                mousedown: (e: MouseEvent) => {
                  e.preventDefault();
                  const parts = input.value.split(',');
                  parts[parts.length - 1] = ` ${formatAddress(a)}`;
                  input.value = parts.join(',').replace(/^\s+/, '') + ', ';
                  onInput(input.value);
                  list.replaceChildren();
                },
              },
            }, formatAddress(a)),
          ),
        );
      } catch {
        /* suggestions are best-effort */
      }
    }, 150);
  });
  input.addEventListener('blur', () => setTimeout(() => list.replaceChildren(), 150));
  return h('label.field-row', h('span.field-label', label), h('div.field-input', input, list));
}

async function fileToAttachment(f: File): Promise<OutgoingAttachment> {
  const buf = new Uint8Array(await f.arrayBuffer());
  let bin = '';
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return { filename: f.name, mime_type: f.type || 'application/octet-stream', data_base64: btoa(bin) };
}

export function openComposer(initial: Partial<Draft> & { account_id?: string }, opts: { title?: string } = {}) {
  if (open) open.close();
  const accounts = store.state.accounts;
  if (!accounts.length) {
    toast('Add an account in Settings first.', { kind: 'error' });
    return;
  }
  const draft: Draft = {
    id: initial.id ?? null,
    account_id: initial.account_id ?? accounts[0].id,
    to: initial.to ?? [],
    cc: initial.cc ?? [],
    bcc: initial.bcc ?? [],
    subject: initial.subject ?? '',
    body_text: initial.body_text ?? '',
    body_html: null,
    in_reply_to: initial.in_reply_to ?? null,
    forward_of: initial.forward_of ?? null,
    attachments: initial.attachments ?? [],
  };
  let to = draft.to.map(formatAddress).join(', ');
  let cc = draft.cc.map(formatAddress).join(', ');
  let bcc = draft.bcc.map(formatAddress).join(', ');
  let dirty = false;
  let saving: Promise<void> | null = null;

  const collect = (): Draft => ({ ...draft, to: parseAddresses(to), cc: parseAddresses(cc), bcc: parseAddresses(bcc) });

  const save = async () => {
    if (!dirty) return;
    dirty = false;
    saving = (async () => {
      try {
        const r = await api.saveDraft(collect());
        draft.id = r.draft_id;
        status.textContent = 'Draft saved';
      } catch (e) {
        status.textContent = 'Could not save draft';
        console.warn(e);
      }
    })();
    await saving;
  };
  const autosave = window.setInterval(() => void save(), 4000);
  const touch = () => {
    dirty = true;
    status.textContent = '';
  };

  const accountSelect = h(
    'select',
    { 'aria-label': 'From', on: { change: () => { draft.account_id = accountSelect.value; touch(); } } },
    accounts.map((a) => h('option', { value: a.id, selected: a.id === draft.account_id }, `${a.display_name || a.email} <${a.email}>`)),
  );
  const subject = h('input.subject', {
    type: 'text',
    value: draft.subject,
    placeholder: 'Subject',
    'aria-label': 'Subject',
    on: { input: () => { draft.subject = subject.value; touch(); } },
  });
  const body = h('textarea.body', {
    'aria-label': 'Message',
    spellcheck: true,
    on: { input: () => { draft.body_text = body.value; touch(); } },
  });
  body.value = draft.body_text;
  const showCc = cc || bcc;
  const ccRow = addrField('Cc', cc, (v) => { cc = v; touch(); });
  const bccRow = addrField('Bcc', bcc, (v) => { bcc = v; touch(); });
  if (!showCc) {
    ccRow.hidden = true;
    bccRow.hidden = true;
  }
  const attachList = h('div.attach-list');
  const renderAttachments = () =>
    attachList.replaceChildren(
      ...draft.attachments.map((a, i) =>
        h('span.chip', icon('paperclip', 14), `${a.filename} (${humanSize(Math.round((a.data_base64.length * 3) / 4))})`,
          h('button.icon-btn', { 'aria-label': `Remove ${a.filename}`, on: { click: () => { draft.attachments.splice(i, 1); touch(); renderAttachments(); } } }, icon('close', 12))),
      ),
    );
  renderAttachments();
  const fileInput = h('input', { type: 'file', multiple: true, hidden: true });
  fileInput.addEventListener('change', async () => {
    for (const f of fileInput.files ?? []) {
      if (f.size > 25 * 1024 * 1024) {
        toast(`${f.name} is larger than 25 MB`, { kind: 'error' });
        continue;
      }
      draft.attachments.push(await fileToAttachment(f));
    }
    fileInput.value = '';
    touch();
    renderAttachments();
  });
  const status = h('span.muted.small');

  const close = () => {
    clearInterval(autosave);
    panel.remove();
    document.removeEventListener('keydown', onKey, true);
    open = null;
  };
  const discard = async () => {
    if ((draft.body_text.trim() || draft.subject.trim()) && !(await confirmDialog('Discard draft?', 'This draft will be deleted.', 'Discard', true))) return;
    clearInterval(autosave);
    if (draft.id) await api.deleteDraft(draft.id).catch(() => {});
    close();
  };
  const send = async () => {
    const d = collect();
    if (!d.to.length && !d.cc.length && !d.bcc.length) {
      toast('Add at least one recipient.', { kind: 'error' });
      return;
    }
    if (!d.subject.trim() && !(await confirmDialog('Send without a subject?', 'This message has no subject.', 'Send'))) return;
    await saving;
    if (dirty) await save();
    d.id = draft.id;
    close();
    let cancelled = false;
    toast(`Sending in ${UNDO_SECONDS} seconds…`, {
      ms: UNDO_SECONDS * 1000,
      action: {
        label: 'Undo',
        run: () => {
          cancelled = true;
          openComposer(d, opts);
        },
      },
    });
    setTimeout(async () => {
      if (cancelled) return;
      try {
        await api.send(d);
        toast('Sent', { kind: 'success' });
      } catch (e) {
        errorToast(e);
        openComposer(d, opts);
      }
    }, UNDO_SECONDS * 1000);
  };
  const onKey = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void send();
    } else if (e.key === 'Escape' && panel.contains(document.activeElement)) {
      e.preventDefault();
      void save().then(close);
    }
  };
  document.addEventListener('keydown', onKey, true);

  const panel = h(
    'section.composer',
    { role: 'dialog', 'aria-label': opts.title ?? 'New message' },
    h('header.composer-head',
      h('h2', opts.title ?? (draft.in_reply_to ? 'Reply' : draft.forward_of ? 'Forward' : 'New message')),
      h('button.icon-btn', { 'aria-label': 'Close (keeps draft)', title: 'Close (keeps draft)', on: { click: () => void save().then(close) } }, icon('close'))),
    h('div.composer-fields',
      h('label.field-row', h('span.field-label', 'From'), h('div.field-input', accountSelect)),
      h('div.field-row-wrap',
        addrField('To', to, (v) => { to = v; touch(); }),
        !showCc && h('button.link.small', { type: 'button', on: { click: (e: MouseEvent) => { ccRow.hidden = false; bccRow.hidden = false; (e.target as HTMLElement).remove(); } } }, 'Cc/Bcc')),
      ccRow,
      bccRow,
      h('label.field-row', h('span.field-label', 'Subject'), h('div.field-input', subject))),
    body,
    attachList,
    h('footer.composer-foot',
      h('button.primary', { on: { click: () => void send() }, title: 'Send (⌘↩)' }, icon('send', 16), 'Send'),
      h('button.icon-btn', { 'aria-label': 'Attach files', title: 'Attach files', on: { click: () => fileInput.click() } }, icon('paperclip')),
      fileInput,
      status,
      h('span.spacer'),
      h('button.icon-btn', { 'aria-label': 'Discard draft', title: 'Discard', on: { click: () => void discard() } }, icon('trash'))),
  );
  document.body.appendChild(panel);
  open = { close };
  requestAnimationFrame(() => {
    if (!to) (panel.querySelector('.field-input input') as HTMLInputElement | null)?.focus();
    else {
      body.focus();
      body.setSelectionRange(0, 0);
      body.scrollTop = 0;
    }
  });
}
