// The What's New dialog: CHANGELOG.md, bundled into the app at build time
// (0082), so it opens without a browser or a network. A version waiting to be
// installed isn't in this build's copy yet; its notes come from the update feed.

import source from '../../../CHANGELOG.md?raw';
import { h } from '../lib/dom';
import { modal } from '../lib/ui';
import { openExternal } from '../views/mail/message-body';
import { type Block, niceDate, parseBlocks, parseChangelog, parseInline, type Release } from './changelog-notes';

/** The same notes on the site, for sharing or reading elsewhere. */
export const CHANGELOG_URL = 'https://satchel.email/changelog';

let releases: Release[] | null = null;

function inline(text: string): Node[] {
  return parseInline(text).map((s) => {
    if (s.code) return h('code', s.text);
    if (s.bold) return h('b', s.text);
    const href = s.href;
    if (href) return h('a', { href, title: href, on: { click: (e: Event) => { e.preventDefault(); openExternal(href); } } }, s.text);
    return document.createTextNode(s.text);
  });
}

function block(b: Block): HTMLElement {
  return b.kind === 'p' ? h('p', ...inline(b.text)) : h('ul', ...b.items.map((i) => h('li', ...inline(i))));
}

function section(r: Release, note: string | null, current: boolean): HTMLElement {
  return h(
    `section.release${current ? '.current' : ''}`,
    { 'data-version': r.version },
    h('h3', r.version, note ? h('span.muted', note) : r.date ? h('span.muted', niceDate(r.date)) : null),
    ...r.blocks.map(block),
  );
}

/** Opens What's New, scrolled to `version` if given. `upcoming` is a version
 *  downloaded but not yet installed, with its notes from the update feed. */
export function showChangelog(opts: { version?: string; upcoming?: { version: string; notes: string } } = {}) {
  releases ??= parseChangelog(source);
  const list = [...releases];
  const up = opts.upcoming;
  const upcoming = up && !list.some((r) => r.version === up.version) && up.notes.trim()
    ? { version: up.version, date: '', blocks: parseBlocks(up.notes) }
    : null;
  const sections = [
    ...(upcoming ? [section(upcoming, 'Ready to install', upcoming.version === opts.version)] : []),
    ...list.map((r) => section(r, null, r.version === opts.version)),
  ];
  const m = modal('What’s new', h('div.changelog', ...sections), {
    wide: true,
    footer: h('div.row', h('span.muted.small', 'Also on the web:'), h('button.link.small', { title: 'Open satchel.email/changelog in your browser', on: { click: () => openExternal(CHANGELOG_URL) } }, 'satchel.email/changelog')),
  });
  const target = opts.version && m.body.querySelector<HTMLElement>(`[data-version="${CSS.escape(opts.version)}"]`);
  if (target) requestAnimationFrame(() => {
    m.body.scrollTop += target.getBoundingClientRect().top - m.body.getBoundingClientRect().top;
  });
}
