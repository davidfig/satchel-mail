// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { fromWall, offsetLabel, offsetMinutes, toWall } from '../src/lib/tz.ts';

describe('tz', () => {
  test('offsets follow daylight saving', () => {
    assert.equal(offsetMinutes('America/Los_Angeles', new Date('2026-01-15T12:00:00Z')), -480);
    assert.equal(offsetMinutes('America/Los_Angeles', new Date('2026-07-15T12:00:00Z')), -420);
    assert.equal(offsetMinutes('Asia/Kolkata', new Date('2026-07-15T12:00:00Z')), 330);
  });

  test('wall time round-trips', () => {
    const at = fromWall('2026-10-30', '09:30', 'Europe/Berlin');
    assert.equal(at.toISOString(), '2026-10-30T08:30:00.000Z');
    assert.deepEqual(toWall(at, 'Europe/Berlin'), { date: '2026-10-30', time: '09:30' });
    assert.deepEqual(toWall(at, 'America/New_York'), { date: '2026-10-30', time: '04:30' });
  });

  test('a time skipped by DST lands just after the jump', () => {
    // 2:30 AM doesn't exist in Los Angeles on 2026-03-08.
    assert.deepEqual(toWall(fromWall('2026-03-08', '02:30', 'America/Los_Angeles'), 'America/Los_Angeles'), { date: '2026-03-08', time: '03:30' });
  });

  test('a repeated time takes the first one', () => {
    // 1:30 AM happens twice in Los Angeles on 2026-11-01; the first is PDT.
    assert.equal(fromWall('2026-11-01', '01:30', 'America/Los_Angeles').toISOString(), '2026-11-01T08:30:00.000Z');
  });

  test('labels offsets', () => {
    assert.equal(offsetLabel('UTC'), 'GMT');
    assert.equal(offsetLabel('Asia/Kolkata'), 'GMT+5:30');
    assert.equal(offsetLabel('America/Los_Angeles', new Date('2026-01-15T12:00:00Z')), 'GMT-8');
  });
});
