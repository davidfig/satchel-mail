// Attachments: the chips under a message, and a viewer that shows the kinds of
// file the webview can draw itself (images, PDFs, audio and video, text) in
// the app, with Open (in its default app, desktop only) and Save beside it (0054).

import { api } from '../../api';
import { h, icon, patch } from '../../lib/dom';
import { humanSize } from '../../lib/format';
import { confirmDialog, contextMenu, errorToast, modal, toast, type MenuItem } from '../../lib/ui';
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
        on: { click: () => openViewer(m, a) },
      }, icon(a.risky ? 'warning' : 'paperclip', 16), h('span.name', a.filename), h('span.muted.small', humanSize(a.size)));
      contextMenu(chip, () => fileMenu(m, a));
      return chip;
    }));
}

function fileMenu(m: MessageDetail, a: Attachment): MenuItem[] {
  const items: MenuItem[] = [{ label: 'View', icon: 'image', run: () => openViewer(m, a) }];
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

/** Shows `start` full-window, with the message's other attachments a ←/→ away. */
export function openViewer(m: MessageDetail, start: Attachment) {
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
  const onKey = (e: KeyboardEvent) => {
    if (document.querySelector('.menu') || (e.target as HTMLElement).closest?.('input, textarea')) return;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      go(e.key === 'ArrowLeft' ? -1 : 1);
    }
  };
  const many = list.length > 1;
  const actions = h('div.viewer-actions',
    many && h('span.muted.small.viewer-count'),
    many && h('button.icon-btn', { title: 'Previous attachment (←)', on: { click: () => go(-1) } }, icon('chevronLeft')),
    many && h('button.icon-btn', { title: 'Next attachment (→)', on: { click: () => go(1) } }, icon('chevronRight')),
    transport.kind === 'tauri' && h('button.icon-btn', { title: 'Open in its app', on: { click: () => void openFile(m.id, current()) } }, icon('external')),
    h('button.icon-btn', { title: 'Save…', on: { click: () => void saveFile(m.id, current()) } }, icon('download')),
  );
  const body = h('div.viewer');
  const handle = modal(start.filename, body, {
    full: true,
    actions,
    onClose: () => {
      document.removeEventListener('keydown', onKey);
      if (url) URL.revokeObjectURL(url);
    },
  });
  document.addEventListener('keydown', onKey);
  const heading = handle.root.querySelector('.modal-head h2') as HTMLElement;

  async function show() {
    const a = current();
    const mine = ++seq;
    heading.textContent = a.filename;
    const count = actions.querySelector('.viewer-count');
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
    if (kind === 'pdf') return patch(body, h('iframe.viewer-frame', { src: url, title: a.filename }));
    if (kind === 'image') return patch(body, h('div.viewer-media', h('img', { src: url, alt: a.filename, on: { error: failed } })));
    patch(body, h('div.viewer-media', h(kind, { src: url, controls: true, on: { error: failed } })));
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
