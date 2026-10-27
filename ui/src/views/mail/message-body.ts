// Message bodies: text and HTML rendering, invitations, attachments, links.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { humanSize, timeRange } from '../../lib/format';
import { confirmDialog, errorToast, modal, toast } from '../../lib/ui';
import { transport } from '../../transport';
import type { MessageDetail, PartStat } from '../../types';
import { isAttribution, trimUrl } from '../../lib/thread';
import { editDraft, openComposer } from './composer';

export function attachments(m: MessageDetail): HTMLElement | null {
  const atts = m.attachments.filter((a) => !a.inline);
  if (!atts.length) return null;
  return h('div.attachments',
    atts.map((a) =>
      h(`button.attachment${a.risky ? '.risky' : ''}`, {
        title: a.risky ? `${a.filename} can run code or imitate a sign-in page` : `Download ${a.filename}`,
        on: {
          click: async () => {
            if (a.risky && !(await confirmDialog('Download a risky file?', `${a.filename} can run code or imitate a sign-in page when opened. Only download it if you were expecting it from this sender.`, 'Download anyway', true))) return;
            void download(m.id, a.id);
          },
        },
      }, icon(a.risky ? 'warning' : 'paperclip', 16), h('span.name', a.filename), h('span.muted.small', humanSize(a.size)))));
}

/** The core's phishing and safety warnings for one message. */
export function warnings(m: MessageDetail): HTMLElement | null {
  const list = m.warnings ?? [];
  if (!list.length) return null;
  return h('div.banner.warn.msg-warnings', { role: 'note' },
    icon('warning', 16),
    h('div.grow', h('strong', list.some((w) => w.kind !== 'unknown_inviter' && w.kind !== 'risky_attachment') ? 'This might be a scam' : 'Be careful'),
      list.map((w) => h('div', w.message))));
}

export function inviteCard(m: MessageDetail, inSpam: boolean): HTMLElement {
  const inv = m.invite!;
  const when = `${new Date(inv.start).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })} · ${timeRange(inv.start, inv.end, inv.all_day)}`;
  if (inSpam && inv.method === 'REQUEST') {
    // Any reply, even a decline, confirms the address to a spammer.
    return h('div.invite.muted-invite', icon('calendar', 20), h('div', h('strong', inv.title), h('div.muted.small', when), h('div.muted.small', 'Invitations in Spam can’t be answered. Move this to the Inbox first if it’s real.')));
  }
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

/** Open a link from a message; `cautious` ones show where they really go first. */
async function openLink(url: string, cautious: boolean) {
  if (cautious) {
    let host = url;
    try {
      host = new URL(url).host;
    } catch {
      /* show the raw link */
    }
    if (!(await confirmDialog('Open this link?', `It goes to ${host}. Mail in Spam or with warnings often hides where links lead, so check that's where you meant to go.`, 'Open link'))) return;
  }
  openExternal(url);
}

function linkified(line: string, cautious: boolean): HTMLElement {
  const div = h(line.startsWith('>') ? 'div.quote' : 'div');
  for (const p of line.split(/(https?:\/\/[^\s<>"]+)/g)) {
    if (/^https?:\/\//.test(p)) {
      const [url, rest] = trimUrl(p);
      div.appendChild(h('a', { href: url, rel: 'noopener noreferrer', target: '_blank', on: { click: (e: MouseEvent) => { e.preventDefault(); void openLink(url, cautious); } } }, url));
      if (rest) div.appendChild(document.createTextNode(rest));
    } else if (p) div.appendChild(document.createTextNode(p));
  }
  if (!line) div.appendChild(h('br'));
  return div;
}

export function draftBanner(m: MessageDetail): HTMLElement {
  return h('div.banner',
    icon('draft', 16),
    h('span.grow', 'Draft, not sent.'),
    h('button.link', { title: 'Open this draft in the composer', on: { click: () => void editDraft(m.id) } }, 'Edit draft'));
}

/** Plain text body: linkified, with quoted history folded behind a toggle. */
export function textBody(text: string, cautious: boolean): HTMLElement {
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
        const block = h('div.quoted', { hidden: true }, lines.slice(start, i).map((l) => linkified(l, cautious)));
        pre.append(h('button.quote-toggle', { title: 'Show quoted text', 'aria-label': 'Show quoted text', on: { click: (e: MouseEvent) => { block.hidden = !block.hidden; (e.currentTarget as HTMLElement).classList.toggle('open'); } } }, '•••'), block);
        continue;
      }
      i = start;
    }
    pre.appendChild(linkified(line, cautious));
    i++;
  }
  return pre;
}

/** Sanitized HTML (from the core) in a script-less sandboxed iframe with a CSP.
 *  Without `allowRemote` the CSP itself blocks remote images: CSS escapes
 *  and `image-set()` can slip past the sanitizer's checks. */
export function htmlFrame(html: string, cautious: boolean, allowRemote: boolean): HTMLElement {
  const img = allowRemote ? 'data: https: http:' : 'data:';
  const csp = `default-src 'none'; img-src ${img}; style-src 'unsafe-inline'; font-src data:`;
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
        } else void openLink(a.href, cautious);
      }
    });
  });
  return h('div.html-wrap', frame);
}

export function openExternal(url: string) {
  if (transport.kind === 'tauri') void transport.host('open_url', { url }).catch(() => window.open(url, '_blank', 'noopener'));
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
    const path = await transport.host<string | null>('save_file', { filename, data_base64: b64 });
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
