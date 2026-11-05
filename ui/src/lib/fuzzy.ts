// Fuzzy matching for pickers: the query's characters, in order, anywhere in
// the text, so "gti" finds "Go to Inbox". Scores favour whole substrings,
// word starts and runs; `at` holds the matched positions for highlighting.

export interface Match {
  score: number;
  at: number[];
}

const wordStart = (t: string, i: number) => i === 0 || !/[a-z0-9]/i.test(t[i - 1]);

export function fuzzy(query: string, text: string): Match | null {
  const q = query.toLowerCase().replace(/\s+/g, ' ').trim();
  if (!q) return { score: 0, at: [] };
  const t = text.toLowerCase();
  const sub = t.indexOf(q);
  if (sub >= 0) {
    // A word-start occurrence beats an earlier one mid-word ("box" in "Inbox, Boxes").
    let i = sub;
    while (i >= 0 && !wordStart(t, i)) i = t.indexOf(q, i + 1);
    const at = i >= 0 ? i : sub;
    return { score: 100 + q.length * 4 + (wordStart(t, at) ? 20 : 0) + (at === 0 ? 10 : 0) - at * 0.1, at: range(at, q.length) };
  }
  const chars = q.replace(/ /g, '');
  const at = scatter(chars, t, true) ?? scatter(chars, t, false);
  if (!at) return null;
  let score = 0;
  at.forEach((p, k) => {
    score += 1 + (wordStart(t, p) ? 4 : 0) + (k > 0 && at[k - 1] === p - 1 ? 3 : 0);
  });
  return { score: score - (at[at.length - 1] - at[0]) * 0.05, at };
}

/** Matches left to right, continuing a run when it can and otherwise
 *  jumping to the next word start (`words`), or to the next occurrence. */
function scatter(q: string, t: string, words: boolean): number[] | null {
  const at: number[] = [];
  let from = 0;
  for (const c of q) {
    const prev = at[at.length - 1];
    let i = prev !== undefined && t[prev + 1] === c ? prev + 1 : -1;
    if (i < 0 && words) for (let j = t.indexOf(c, from); j >= 0 && i < 0; j = t.indexOf(c, j + 1)) if (wordStart(t, j)) i = j;
    if (i < 0) i = t.indexOf(c, from);
    if (i < 0) return null;
    at.push(i);
    from = i + 1;
  }
  return at;
}

function range(start: number, n: number) {
  return Array.from({ length: n }, (_, k) => start + k);
}
