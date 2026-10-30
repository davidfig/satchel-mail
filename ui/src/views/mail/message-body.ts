// Message bodies: text and HTML rendering, invitations, links.

import { api } from '../../api';
import { h, icon, patch } from '../../lib/dom';
import { timeRange } from '../../lib/format';
import { confirmDialog, errorToast, modal, toast } from '../../lib/ui';
import { transport } from '../../transport';
import type { MessageDetail, PartStat } from '../../types';
import { followTarget, isAttribution, linkScheme, routeLinks, trimUrl } from '../../lib/thread';
import { editDraft, openComposer } from './composer';

/** The core's phishing and safety warnings for one message. A different
 *  Reply-To only matters when answering, so the reply box shows that one
 *  (`replyTo: true`) and the message leaves it out. */
export function warnings(m: MessageDetail, replyTo = false): HTMLElement | null {
  const list = (m.warnings ?? []).filter((w) => (w.kind === 'reply_to') === replyTo);
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
    patch(card,
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
  if (await confirmLink(url, cautious)) openExternal(url);
}

/** Asks before opening `url` when `cautious` or `always`, saying where it really goes.
 *  `always` is for a link that came from outside (the `#follow` page): it also
 *  shows the whole address. */
export async function confirmLink(url: string, cautious: boolean, always = false): Promise<boolean> {
  if (!cautious && !always) return true;
  let host = url;
  try {
    host = new URL(url).host;
  } catch {
    /* show the raw link */
  }
  const why = cautious
    ? `It goes to ${host}. Mail in Spam or with warnings often hides where links lead, so check that's where you meant to go.`
    : `It goes to ${host}. Check that's where you meant to go: any website can send you to this page with a link.`;
  if (!always) return confirmDialog('Open this link?', why, 'Open link');
  return confirmDialog('Open this link?', h('div', h('p', why), h('p.link-address', url)), 'Open link');
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
    img{max-width:100%} img[width]{height:auto} table{max-width:100%} blockquote{border-left:3px solid #ccc;margin:0 0 0 .5em;padding-left:.75em;color:#555}
    a{color:#2a62d9}
  </style></head><body>${transport.kind === 'http' ? routeLinks(html, location.origin, cautious) : html}</body></html>`;
  // `allow-popups` stays: on desktop a link reaches the host as a popup, which
  // it refuses and hands to `catchHostLinks`. On the web every link points at
  // our own `#follow` page (`routeLinks`); WebKit runs no listeners here, so the
  // click opens that page as a popup, which must escape the sandbox to run
  // scripts and ask before going on.
  const sandbox = transport.kind === 'tauri' ? 'allow-same-origin allow-popups' : 'allow-same-origin allow-popups allow-popups-to-escape-sandbox';
  const frame = h('iframe.html-body', { sandbox, referrerpolicy: 'no-referrer', title: 'Message content', dataset: { cautious: String(cautious) } }) as HTMLIFrameElement;
  frame.srcdoc = doc;
  frame.addEventListener('load', () => {
    const d = frame.contentDocument;
    if (!d) return;
    const fit = () => (frame.style.height = `${Math.min(d.documentElement.scrollHeight + 4, 20000)}px`);
    fit();
    new ResizeObserver(fit).observe(d.body);
    // Chromium and Firefox run this; WebKit runs no listeners in a script-less
    // sandboxed frame, so there the link opens as a popup: on desktop
    // `catchHostLinks` takes it, on the web it is our `#follow` page.
    const onLink = (e: MouseEvent) => {
      const a = (e.target as HTMLElement).closest('a');
      // Anchors within the message scroll as usual. Anything else is stopped
      // before we look at it, so a malformed link can't slip past by throwing.
      if (!a?.href || a.href.startsWith('about:srcdoc#')) return;
      e.preventDefault();
      // On the web the link was routed through `#follow`; check what it carries.
      followLink(followTarget(a.href, location.origin)?.url ?? a.href, cautious);
    };
    d.addEventListener('click', onLink);
    // Middle-click opens a tab without a `click`; it gets the same check.
    d.addEventListener('auxclick', (e) => {
      if (e.button === 1) onLink(e);
    });
  });
  return h('div.html-wrap', frame);
}

/** Opens a link from a message body; false if it isn't one we follow. */
export function followLink(href: string, cautious: boolean): boolean {
  const scheme = linkScheme(href);
  if (scheme === 'mail') {
    const email = mailtoAddress(href);
    if (email) openComposer({ to: [{ email }] });
    else toast('This email link is malformed, so Satchel didn’t open it.');
  } else if (scheme === 'web') void openLink(href, cautious);
  else return false;
  return true;
}

/** The address of a mailto: link, or null if it doesn't decode. */
function mailtoAddress(href: string): string | null {
  try {
    return decodeURIComponent(href.slice(7).split('?')[0]).trim() || null;
  } catch {
    return null;
  }
}

/** Desktop: a link clicked in a frame reaches us from the host, which caught
 *  it as a popup (a message iframe) or a navigation (a PDF attachment). The
 *  click focused that iframe, which says whether to be cautious; if we can't
 *  tell, we are. Installed once at startup. */
export function catchHostLinks() {
  transport.onOpenLink((url) => {
    const frame = document.activeElement;
    const cautious = !(frame instanceof HTMLIFrameElement && frame.dataset.cautious === 'false');
    followLink(url, cautious);
  });
}

export function openExternal(url: string) {
  // No window.open fallback on desktop: the webview can't show it, and the
  // host would hand it straight back here.
  if (transport.kind === 'tauri') void transport.host('open_url', { url }).catch(errorToast);
  else window.open(url, '_blank', 'noopener,noreferrer');
}

export async function viewSource(messageId: string) {
  try {
    const { raw } = await api.getRaw(messageId);
    modal('Message source', h('pre.source', raw), { wide: true });
  } catch (e) {
    errorToast(e);
  }
}
