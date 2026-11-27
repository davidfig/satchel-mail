// Undoing the hard wrap in plain-text mail. Many clients break lines at about
// 72 characters; shown as-is in a narrower reader, every line wraps twice and
// leaves ragged gaps. We rejoin the breaks the wrapping made and keep the ones
// the sender typed.

/** Narrower than this, short lines are probably meant (a poem, an address). */
const MIN_WIDTH = 50;
/** Wider than this, the sender's client didn't hard-wrap. */
const MAX_WIDTH = 100;

const LIST_ITEM = /^\s*([-*•+]|\d{1,3}[.)])\s/;

/** `lines` with each hard-wrapped paragraph joined into one line. A break is
 *  a wrap when the next line's first word wouldn't have fit on this one. */
export function unwrap(lines: string[]): string[] {
  const width = wrapWidth(lines);
  if (!width) return lines;
  const out: string[] = [];
  // The last original line of `out`'s last entry, which may be joined.
  let last: string | undefined;
  for (const line of lines) {
    if (last !== undefined && wrapped(last, line, width)) out[out.length - 1] = `${out[out.length - 1].trimEnd()} ${line.trimStart()}`;
    else out.push(line);
    last = line;
  }
  return out;
}

/** The width the text was wrapped at: its longest line with a space in it
 *  (a long URL alone on a line doesn't count), or 0 if it wasn't wrapped. */
function wrapWidth(lines: string[]): number {
  let width = 0;
  for (const l of lines) if (!l.startsWith('>') && /\S\s+\S/.test(l)) width = Math.max(width, l.trimEnd().length);
  return width >= MIN_WIDTH && width <= MAX_WIDTH ? width : 0;
}

/** Whether the break between `last` and `line` came from wrapping. */
function wrapped(last: string, line: string, width: number): boolean {
  if (!last.trim() || !line.trim()) return false;
  if (last.startsWith('>') || line.startsWith('>')) return false;
  if (/^\s/.test(line) || LIST_ITEM.test(line)) return false;
  if (last.trimEnd() === '--' || line.trimEnd() === '--') return false;
  const word = line.trimStart().split(/\s/, 1)[0];
  return last.trimEnd().length + 1 + word.length > width;
}
