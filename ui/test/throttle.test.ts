import assert from 'node:assert/strict';
import { test } from 'node:test';
import { throttled } from '../src/lib/throttle.ts';

test('runs at most once per interval, counting from the start', () => {
  let time = 1000;
  let runs = 0;
  const f = throttled(() => runs++, 100, () => time);
  f();
  assert.equal(runs, 0);
  time += 100;
  f();
  assert.equal(runs, 1);
  time += 99;
  f();
  assert.equal(runs, 1);
  time += 1;
  f();
  assert.equal(runs, 2);
});
