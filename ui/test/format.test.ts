// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { formatAddress, parseAddresses, shortName, shortProgress } from '../src/lib/format.ts';

describe('addresses', () => {
  test('names with commas survive a round trip', () => {
    const list = [{ name: 'Doe, John', email: 'john@x.com' }, { name: 'Ann "A" Lee', email: 'ann@x.com' }, { name: 'Bob', email: 'bob@x.com' }];
    assert.deepEqual(parseAddresses(list.map(formatAddress).join(', ')), list);
  });

  test('an unquoted "Last, First <address>" stays one address', () => {
    assert.deepEqual(parseAddresses('Doe, John <john@x.com>; c@d.com'), [{ name: 'Doe, John', email: 'john@x.com' }, { email: 'c@d.com' }]);
  });

  test('plain addresses and typos stay separate', () => {
    assert.deepEqual(parseAddresses('a@b.com, bob, c@d.com\ne@f.com'), [{ email: 'a@b.com' }, { email: 'bob' }, { email: 'c@d.com' }, { email: 'e@f.com' }]);
  });

  test('short names read "Last, First" the right way round', () => {
    assert.equal(shortName({ name: 'Doe, John', email: 'j@x.com' }), 'John');
    assert.equal(shortName({ name: 'Ann Lee', email: 'a@x.com' }), 'Ann');
    assert.equal(shortName({ name: null, email: 'bob@x.com' }), 'bob');
  });
});

describe('sync progress', () => {
  test('short form drops the noun', () => {
    assert.equal(shortProgress('Downloading messages: 31,295 left'), 'Downloading: 31,295 left');
    assert.equal(shortProgress('Downloading headers 1,500/4,000'), 'Downloading 1,500/4,000');
    assert.equal(shortProgress('INBOX: 1234 messages'), 'INBOX: 1234');
    assert.equal(shortProgress('Older mail 10/20'), 'Older mail 10/20');
  });
});
