// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { describeRule, formatRule, nthOf, parseRule, untilDate } from '../src/lib/rrule.ts';

// Friday, October 30, 2026: the last Friday of its month.
const fri = new Date(2026, 9, 30);
const say = (rule: string, until?: string) => describeRule(parseRule(rule)!, fri, until);

describe('rrule', () => {
  test('round-trips the parts it edits and keeps the rest', () => {
    const text = 'FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10;WKST=SU';
    assert.equal(formatRule(parseRule(text)!), text);
    assert.equal(formatRule(parseRule('FREQ=MONTHLY;BYDAY=-1FR')!), 'FREQ=MONTHLY;BYDAY=-1FR');
  });

  test('refuses what it can’t show', () => {
    assert.equal(parseRule('FREQ=HOURLY'), null);
    assert.equal(parseRule('FREQ=MONTHLY;BYDAY=1MO,-1FR'), null);
  });

  test('describes rules in words', () => {
    assert.equal(say('FREQ=DAILY'), 'Every day');
    assert.equal(say('FREQ=DAILY;INTERVAL=3'), 'Every 3 days');
    assert.equal(say('FREQ=WEEKLY'), 'Every week on Friday');
    assert.equal(say('FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE'), 'Every 2 weeks on Monday and Wednesday');
    assert.equal(say('FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR'), 'Every weekday');
    assert.equal(say('FREQ=WEEKLY;BYDAY=MO,WE,FR'), 'Every week on Mon, Wed, and Fri');
    assert.equal(say('FREQ=MONTHLY'), 'Every month on day 30');
    assert.equal(say('FREQ=MONTHLY;BYDAY=-1FR'), 'Every month on the last Friday');
    assert.equal(say('FREQ=YEARLY'), 'Every year on October 30');
    assert.equal(say('FREQ=WEEKLY;COUNT=5'), 'Every week on Friday, 5 times');
    assert.equal(say('FREQ=DAILY;UNTIL=20261231T000000Z', '2026-12-31'), 'Every day, until Dec 31, 2026');
  });

  test('finds which weekday of the month a date is', () => {
    assert.deepEqual(nthOf(fri), { n: 5, last: true });
    assert.deepEqual(nthOf(new Date(2026, 9, 13)), { n: 2, last: false });
  });

  test('reads UNTIL as a date', () => {
    assert.equal(untilDate('20261231', () => 'x'), '2026-12-31');
    assert.equal(untilDate('20270101T015959Z', (d) => d.toISOString().slice(0, 10)), '2027-01-01');
  });
});
