// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { fuzzy } from '../src/lib/fuzzy.ts';

const rank = (q: string, labels: string[]) =>
  labels.map((l) => ({ l, m: fuzzy(q, l) })).filter((x) => x.m).sort((a, b) => b.m!.score - a.m!.score).map((x) => x.l);

describe('fuzzy', () => {
  test('matches initials across words', () => {
    assert.deepEqual(fuzzy('gti', 'Go to Inbox')?.at, [0, 3, 6]);
  });

  test('rejects text missing a character', () => {
    assert.equal(fuzzy('inboz', 'Go to Inbox'), null);
  });

  test('an empty query matches everything', () => {
    assert.deepEqual(fuzzy('  ', 'Archive'), { score: 0, at: [] });
  });

  test('a substring at a word start ranks first', () => {
    assert.deepEqual(rank('arch', ['Mark as read, check', 'Search', 'Archive']), ['Archive', 'Search', 'Mark as read, check']);
  });

  test('prefers the word-start occurrence of a substring', () => {
    assert.deepEqual(fuzzy('box', 'Inbox / Boxes')?.at, [8, 9, 10]);
  });

  test('falls back when word starts lead nowhere', () => {
    assert.deepEqual(fuzzy('ox', 'Box oa')?.at, [1, 2]);
  });
});
