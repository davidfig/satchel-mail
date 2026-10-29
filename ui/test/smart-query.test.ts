// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';

const { emptyFields, formatFields, parseFields, tokens } = await import('../src/lib/smart-query.ts');

describe('smart folder query fields', () => {
  test('fields write a query the core understands', () => {
    const f = { ...emptyFields(), from: 'bob@x.test, carol', subject: 'q3 report', words: 'budget', unread: true, attachment: true, age: '7d', anywhere: true };
    assert.equal(formatFields(f), 'from:bob@x.test,carol subject:"q3 report" is:unread has:attachment newer_than:7d in:anywhere budget');
  });

  test('a typed query fills the fields and keeps what they cannot show', () => {
    const f = parseFields('from:bob is:flagged -from:noreply older_than:1y "due friday" with:dana account:me@x.test', ['me@x.test', 'work@y.test']);
    assert.equal(f.from, 'bob');
    assert.equal(f.flagged, true);
    assert.equal(f.words, '"due friday"');
    assert.deepEqual(f.accounts, ['me@x.test']);
    assert.deepEqual(f.other, ['-from:noreply', 'older_than:1y', 'with:dana']);
  });

  test('round trips', () => {
    const q = 'from:bob to:carol subject:"weekly sync" is:unread newer_than:30d -in:inbox notes';
    assert.equal(formatFields(parseFields(q)), q);
  });

  test('ages the form has no choice for stay in the query', () => {
    const f = parseFields('newer_than:3d');
    assert.equal(f.age, '');
    assert.deepEqual(f.other, ['newer_than:3d']);
  });

  test('an account that does not exist is kept as typed', () => {
    const f = parseFields('account:gone@x.test', ['me@x.test']);
    assert.deepEqual(f.accounts, []);
    assert.deepEqual(f.other, ['account:gone@x.test']);
  });

  test('tokens keep quoted phrases together', () => {
    assert.deepEqual(tokens(' a  "b c" d:"e f" '), ['a', '"b c"', 'd:"e f"']);
  });
});
