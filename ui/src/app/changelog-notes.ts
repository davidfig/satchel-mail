// Reads CHANGELOG.md (0082) for the What's New dialog. Understands what the
// file uses and scripts/changelog.py renders: `## X.Y.Z (YYYY-MM-DD)`
// sections, paragraphs, `- ` lists, `code`, **bold** and [links](url).
// Pure, so it can be tested without a DOM.

export type Block = { kind: 'p'; text: string } | { kind: 'ul'; items: string[] };

export interface Release {
  version: string;
  /** YYYY-MM-DD; empty for notes that came without a heading (the update feed's). */
  date: string;
  blocks: Block[];
}

/** A run of text, set as code, bold or a link. */
export interface Span {
  text: string;
  code?: boolean;
  bold?: boolean;
  href?: string;
}

const HEADING = /^## (\d+\.\d+\.\d+) \((\d{4}-\d{2}-\d{2})\)$/;

/** Every release in the file, newest first as written. */
export function parseChangelog(source: string): Release[] {
  const out: { version: string; date: string; lines: string[] }[] = [];
  for (const line of source.replace(/<!--[\s\S]*?-->/g, '').split(/\r?\n/)) {
    const m = HEADING.exec(line);
    if (m) out.push({ version: m[1], date: m[2], lines: [] });
    else out.at(-1)?.lines.push(line);
  }
  return out.map(({ version, date, lines }) => ({ version, date, blocks: parseBlocks(lines.join('\n')) }));
}

/** One section's body: paragraphs and `- ` lists, split by blank lines. */
export function parseBlocks(body: string): Block[] {
  const blocks: Block[] = [];
  let para: string[] = [];
  let items: string[] = [];
  const flush = () => {
    if (para.length) blocks.push({ kind: 'p', text: para.join(' ') });
    if (items.length) blocks.push({ kind: 'ul', items });
    para = [];
    items = [];
  };
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith('- ')) {
      if (para.length) flush();
      items.push(line.slice(2));
    } else if (!line) flush();
    else if (items.length) items[items.length - 1] += ` ${line}`;
    else para.push(line);
  }
  flush();
  return blocks;
}

const INLINE = /`([^`]+)`|\*\*([^*]+)\*\*|\[([^\]]+)\]\(([^)\s]+)\)/g;

/** Splits a line into plain, code, bold and link runs. Links are kept only if they're http(s). */
export function parseInline(text: string): Span[] {
  const spans: Span[] = [];
  let at = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > at) spans.push({ text: text.slice(at, m.index) });
    if (m[1] !== undefined) spans.push({ text: m[1], code: true });
    else if (m[2] !== undefined) spans.push({ text: m[2], bold: true });
    else spans.push(/^https?:\/\//.test(m[4]) ? { text: m[3], href: m[4] } : { text: m[3] });
    at = m.index + m[0].length;
  }
  if (at < text.length) spans.push({ text: text.slice(at) });
  return spans;
}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

/** 2026-11-15 → November 15, 2026, as on the site. */
export function niceDate(date: string): string {
  const [y, m, d] = date.split('-').map(Number);
  return y && m && d ? `${MONTHS[m - 1]} ${d}, ${y}` : date;
}
