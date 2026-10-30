// Run with `npm test`.

import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { describe, test } from 'node:test';
import type { ThreadSummary } from '../src/types/index.ts';

// The app's imports omit the `.ts` extension (Vite resolves them); do the same here.
registerHooks({
  resolve(spec, ctx, next) {
    try {
      return next(spec, ctx);
    } catch (e) {
      if (spec.startsWith('.') && !spec.endsWith('.ts')) return next(`${spec}.ts`, ctx);
      throw e;
    }
  },
});

// The API module picks a transport at load; nothing here calls it.
Object.assign(globalThis, { window: globalThis, location: { origin: 'http://127.0.0.1' } });
const { ThreadWindow, PAGE } = await import('../src/views/mail/thread-window.ts');

const row = (i: number) => ({ thread_id: `t${i}` }) as ThreadSummary;

/** A window over `total` threads with the given pages cached. */
function windowWith(total: number, pages: number[]) {
  const w = new ThreadWindow(() => ({}) as never, () => {}, () => {});
  w.total = total;
  w.loaded = true;
  const cache = (w as unknown as { pages: Map<number, ThreadSummary[]> }).pages;
  for (const n of pages) cache.set(n, Array.from({ length: Math.min(PAGE, total - n * PAGE) }, (_, k) => row(n * PAGE + k)));
  return w;
}

describe('ThreadWindow.remove', () => {
  test('later rows shift up into the gap', () => {
    const w = windowWith(50, [0]);
    assert.equal(w.remove(new Set(['t3', 't10'])), 2);
    assert.equal(w.total, 48);
    assert.equal(w.at(3)?.thread_id, 't4');
    assert.equal(w.at(9)?.thread_id, 't11');
    assert.equal(w.indexOf('t49'), 47);
  });

  test('a page left short keeps its rows when the rows to fill it are not cached', () => {
    const w = windowWith(PAGE * 3, [0]);
    w.remove(new Set(['t0']));
    assert.equal(w.total, PAGE * 3 - 1);
    assert.equal(w.at(0)?.thread_id, 't1');
    assert.equal(w.at(PAGE - 2)?.thread_id, `t${PAGE - 1}`);
    assert.equal(w.at(PAGE - 1), undefined);
  });

  test('consecutive pages are re-chunked; a page after a gap moves by the rows removed before it', () => {
    const w = windowWith(PAGE * 5, [0, 1, 3]);
    w.remove(new Set(['t5', `t${PAGE * 3 + 1}`]));
    assert.equal(w.at(5)?.thread_id, 't6');
    assert.equal(w.at(PAGE - 1)?.thread_id, `t${PAGE}`);
    // Page 1 now lacks its last row (page 2 isn't cached); the run from page 3
    // moved up one, so its first row (t300, now index 299) is lost.
    assert.equal(w.at(PAGE)?.thread_id, `t${PAGE + 1}`);
    assert.equal(w.at(PAGE * 2 - 1), undefined);
    assert.equal(w.at(PAGE * 3)?.thread_id, `t${PAGE * 3 + 2}`);
    assert.equal(w.total, PAGE * 5 - 2);
  });

  test('threads not in the cache change nothing', () => {
    const w = windowWith(10, [0]);
    assert.equal(w.remove(new Set(['elsewhere'])), 0);
    assert.equal(w.total, 10);
    assert.equal(w.at(9)?.thread_id, 't9');
  });
});
