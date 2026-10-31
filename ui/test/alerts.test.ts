import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ALL_DAY_ALERTS, TIMED_ALERTS, alertLabel } from '../src/lib/alerts.ts';

test('timed alerts in words', () => {
  assert.equal(alertLabel(0, false), 'At start time');
  assert.equal(alertLabel(10, false), '10 minutes before');
  assert.equal(alertLabel(1, false), '1 minute before');
  assert.equal(alertLabel(90, false), '1 hour 30 minutes before');
  assert.equal(alertLabel(2880, false), '2 days before');
  assert.equal(alertLabel(10080, false), '1 week before');
  assert.equal(alertLabel(-5, false), '5 minutes after start');
});

test('all-day alerts count from midnight on the day', () => {
  const [onTheDay, dayBefore, , weekBefore] = ALL_DAY_ALERTS.map((m) => alertLabel(m, true));
  assert.match(onTheDay, /^On the day at 9/);
  assert.match(dayBefore, /^1 day before at 9/);
  assert.match(weekBefore, /^1 week before at 9/);
  assert.ok(TIMED_ALERTS.includes(10));
});
