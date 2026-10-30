// Attachments: the chips under a message, and a viewer that shows the kinds of
// file the webview can draw itself (images, PDFs, audio and video, text) in
// the reading pane, with Open (in its default app, desktop only) and Save beside it (0054).

import { api } from '../../api';
import { h, icon, patch } from '../../lib/dom';
import { humanSize } from '../../lib/format';
import { confirmDialog, contextMenu, errorToast, toast, type MenuItem } from '../../lib/ui';
import { blobType, parseCsv, previewKind } from '../../lib/files';
import { transport } from '../../transport';
import type { Attachment, MessageDetail } from '../../types';

/** Larger text is cut short; the whole file is a Save away. */
const TEXT_LIMIT = 1 << 20;
const ROW_LIMIT = 2000;

export function attachments(m: MessageDetail): HTMLElement | null {
  const atts = m.attachments.filter((a) => !a.inline);
  if (!atts.length) return null;
  return h('div.attachments',
    atts.map((a) => {
      const chip = h(`button.attachment${a.risky ? '.risky' : ''}`, {
        title: a.risky ? `${a.filename} can run code or imitate a sign-in page` : `View ${a.filename}`,
        on: { click: (e: MouseEvent) => openViewer(m, a, e.currentTarget as Element) },
      }, icon(a.risky ? 'warning' : 'paperclip', 16), h('span.name', a.filename), h('span.muted.small', humanSize(a.size)));
      contextMenu(chip, () => fileMenu(m, a, chip));
      return chip;
    }));
}

function fileMenu(m: MessageDetail, a: Attachment, chip: Element): MenuItem[] {
  const items: MenuItem[] = [{ label: 'View', icon: 'image', run: () => openViewer(m, a, chip) }];
  if (transport.kind === 'tauri') items.push({ label: 'Open', icon: 'external', run: () => void openFile(m.id, a) });
  items.push({ label: 'Save…', icon: 'download', run: () => void saveFile(m.id, a) });
  return items;
}

function riskyOk(a: Attachment, verb: 'Open' | 'Download'): Promise<boolean> {
  if (!a.risky) return Promise.resolve(true);
  return confirmDialog(`${verb} a risky file?`, `${a.filename} can run code or imitate a sign-in page when opened. Only ${verb.toLowerCase()} it if you were expecting it from this sender.`, `${verb} anyway`, true);
}

/** Opens the attachment in its default app (desktop). */
async function openFile(messageId: string, a: Attachment) {
  if (!(await riskyOk(a, 'Open'))) return;
  try {
    const f = await api.getAttachment(messageId, a.id);
    await transport.host('open_file', { filename: f.filename, data_base64: f.data_base64 });
  } catch (e) {
    errorToast(e);
  }
}

async function saveFile(messageId: string, a: Attachment) {
  if (!(await riskyOk(a, 'Download'))) return;
  try {
    const f = await api.getAttachment(messageId, a.id);
    await saveBase64(f.filename, f.mime_type, f.data_base64);
  } catch (e) {
    errorToast(e);
  }
}

export async function saveBase64(filename: string, mime: string, b64: string) {
  if (transport.kind === 'tauri') {
    const path = await transport.host<string | null>('save_file', { filename, data_base64: b64 });
    if (path) toast(`Saved to ${path}`, { kind: 'success' });
    return;
  }
  const url = URL.createObjectURL(new Blob([bytes(b64)], { type: mime }));
  const link = h('a', { href: url, download: filename });
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

function bytes(b64: string): Uint8Array<ArrayBuffer> {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Closes the viewer that's open, if any; only one is open at a time. */
let closeOpen: (() => void) | null = null;

/** Shows `start` in place of the conversation in the reading pane that holds
 *  `from` (or the whole window when there's no pane, or after `f`), with the
 *  message's other attachments a ←/→ away and Escape back to the message. */
export function openViewer(m: MessageDetail, start: Attachment, from?: Element | null) {
  closeOpen?.();
  const pane = from?.closest<HTMLElement>('.reader') ?? null;
  const list = m.attachments.filter((a) => !a.inline);
  let index = Math.max(0, list.findIndex((a) => a.id === start.id));
  let url: string | null = null;
  let seq = 0;
  const cache = new Map<string, Promise<string>>();
  const current = () => list[index];
  const go = (step: number) => {
    if (list.length < 2) return;
    index = (index + step + list.length) % list.length;
    void show();
  };
  const many = list.length > 1;
  const heading = h('h2.viewer-title');
  const count = many ? h('span.muted.small.viewer-count') : null;
  const expand = pane && h('button.icon-btn', { title: 'Full window (f)', on: { click: () => toggleExpand() } }, icon('expand'));
  const body = h('div.viewer');
  const root = h('section.viewer-pane', { role: 'dialog', 'aria-label': 'Attachment', tabindex: '-1' },
    h('header.viewer-head',
      heading,
      count,
      many && h('button.icon-btn', { title: 'Previous attachment (←)', on: { click: () => go(-1) } }, icon('chevronLeft')),
      many && h('button.icon-btn', { title: 'Next attachment (→)', on: { click: () => go(1) } }, icon('chevronRight')),
      transport.kind === 'tauri' && h('button.icon-btn', { title: 'Open in its app', on: { click: () => void openFile(m.id, current()) } }, icon('external')),
      h('button.icon-btn', { title: 'Save…', on: { click: () => void saveFile(m.id, current()) } }, icon('download')),
      expand,
      h('button.icon-btn', { title: pane ? 'Back to the message (Esc)' : 'Close (Esc)', on: { click: () => close() } }, icon('close'))),
    body);

  const toggleExpand = () => {
    if (!expand) return;
    const full = root.classList.toggle('expanded');
    patch(expand, icon(full ? 'collapse' : 'expand'));
    expand.dataset.tooltip = full ? 'Back to the reading pane (f)' : 'Full window (f)';
  };
  // The conversation is hidden while the viewer is in its pane (mail.css),
  // not removed, so its frames don't reload; its scroll position comes back.
  const scrolled = pane?.scrollTop ?? 0;
  const prev = document.activeElement as HTMLElement | null;
  const close = () => {
    if (closeOpen !== close) return;
    closeOpen = null;
    document.removeEventListener('keydown', onKey, true);
    if (url) URL.revokeObjectURL(url);
    // Gone already if the pane moved on to another conversation.
    const shown = root.isConnected;
    root.remove();
    if (pane && shown) pane.scrollTop = scrolled;
    if (prev?.isConnected) prev.focus();
  };
  // Ahead of the app's shortcuts. Others (j/k, archive…) still reach the
  // mail view; a new conversation replaces the pane, viewer and all.
  const onKey = (e: KeyboardEvent) => {
    if (!root.isConnected) return close();
    if (document.querySelector('.modal-backdrop, .menu') || (e.target as HTMLElement).closest?.('input, textarea, [contenteditable="true"]')) return;
    const keys: Record<string, () => void> = { Escape: close, ArrowLeft: () => go(-1), ArrowRight: () => go(1), f: toggleExpand };
    const fn = !e.metaKey && !e.ctrlKey && !e.altKey ? keys[e.key] : undefined;
    if (!fn) return;
    e.preventDefault();
    e.stopPropagation();
    fn();
  };
  closeOpen = close;
  document.addEventListener('keydown', onKey, true);
  if (pane) pane.append(root);
  else {
    root.classList.add('expanded');
    document.body.append(root);
  }
  root.focus();

  async function show() {
    const a = current();
    const mine = ++seq;
    heading.textContent = a.filename;
    if (count) count.textContent = `${index + 1} of ${list.length}`;
    if (url) URL.revokeObjectURL(url);
    url = null;
    const kind = previewKind(a);
    if (!kind) return patch(body, noPreview(m, a));
    patch(body, h('p.muted.viewer-loading', 'Loading…'));
    let b64: string;
    try {
      if (!cache.has(a.id)) cache.set(a.id, api.getAttachment(m.id, a.id).then((f) => f.data_base64));
      b64 = await cache.get(a.id)!;
    } catch (e) {
      cache.delete(a.id);
      if (mine === seq) patch(body, noPreview(m, a, 'Satchel couldn’t load this file.'));
      errorToast(e);
      return;
    }
    if (mine !== seq) return;
    const data = bytes(b64);
    if (kind === 'text' || kind === 'csv') {
      const cut = data.length > TEXT_LIMIT;
      const text = new TextDecoder().decode(cut ? data.subarray(0, TEXT_LIMIT) : data);
      const note = cut && h('p.muted.small', `Showing the first ${humanSize(TEXT_LIMIT)}. Save the file to see all of it.`);
      return patch(body, note, kind === 'csv' ? csvTable(text, a.filename.toLowerCase().endsWith('.tsv') ? '\t' : ',') : h('pre.viewer-text', text));
    }
    url = URL.createObjectURL(new Blob([data], { type: blobType(kind, a) }));
    const failed = () => {
      if (mine === seq) patch(body, noPreview(m, a, 'Satchel can’t show this file here.'));
    };
    if (kind === 'pdf') return patch(body, h('iframe.viewer-frame', { src: url, title: a.filename, on: { load: keysFromFrame } }));
    if (kind === 'image') return patch(body, h('div.viewer-media', h('img', { src: url, alt: a.filename, on: { error: failed } })));
    patch(body, h('div.viewer-media', h(kind, { src: url, controls: true, on: { error: failed } })));
  }

  /** Keys typed in the PDF go to its frame, never reaching us; WebKit's viewer
   *  passes them to the frame's window, so Escape and `f` are caught there too
   *  (arrows stay the PDF's). Chromium's viewer keeps them to itself. */
  function keysFromFrame(e: Event) {
    try {
      (e.currentTarget as HTMLIFrameElement).contentWindow?.addEventListener('keydown', (k) => {
        if (k.metaKey || k.ctrlKey || k.altKey) return;
        if (k.key === 'Escape') close();
        else if (k.key === 'f') toggleExpand();
      });
    } catch {
      /* not ours to listen to */
    }
  }

  void show();
}

function noPreview(m: MessageDetail, a: Attachment, why?: string): HTMLElement {
  const reason = why ?? (a.risky ? 'This file can run code or imitate a sign-in page, so Satchel won’t show it.' : 'Satchel can’t show this kind of file.');
  return h('div.viewer-empty',
    icon(a.risky ? 'warning' : 'file', 40),
    h('strong', a.filename),
    h('span.muted.small', humanSize(a.size)),
    h('p.muted', reason),
    h('div.row',
      transport.kind === 'tauri' && h('button.primary', { title: 'Open in its app', on: { click: () => void openFile(m.id, a) } }, icon('external', 16), 'Open'),
      h('button', { title: 'Save…', on: { click: () => void saveFile(m.id, a) } }, icon('download', 16), 'Save…')));
}

function csvTable(text: string, sep: string): HTMLElement {
  const rows = parseCsv(text.replace(/^﻿/, ''), sep);
  const shown = rows.slice(0, ROW_LIMIT);
  const [head, ...rest] = shown;
  return h('div.viewer-table',
    h('table',
      head && h('thead', h('tr', head.map((c) => h('th', c)))),
      h('tbody', rest.map((r) => h('tr', r.map((c) => h('td', c)))))),
    rows.length > ROW_LIMIT && h('p.muted.small', `Showing the first ${ROW_LIMIT} of ${rows.length} rows. Save the file to see all of it.`));
}
