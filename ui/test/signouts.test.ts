import assert from 'node:assert/strict';
import { test } from 'node:test';
import { newlySignedOut, signOutMessage } from '../src/lib/signouts.ts';
import type { Account } from '../src/types/index.ts';

const account = (id: string, state: string, kind = 'gmail') => ({ id, kind, email: `${id}@x.test`, sync: { state } }) as unknown as Account;

test('each sign-out counts once until the account signs back in', () => {
  const seen = new Set<string>();
  assert.deepEqual(newlySignedOut([account('a', 'sign_in'), account('b', 'idle')], seen).map((a) => a.id), ['a']);
  assert.deepEqual(newlySignedOut([account('a', 'sign_in'), account('b', 'idle')], seen), []);
  assert.deepEqual(newlySignedOut([account('a', 'idle')], seen), []);
  assert.deepEqual(newlySignedOut([account('a', 'sign_in')], seen).map((a) => a.id), ['a']);
});

test('other sync problems are not sign-outs', () => {
  assert.deepEqual(newlySignedOut([account('a', 'offline'), account('b', 'error')], new Set()), []);
});

test('the message names who signed the account out', () => {
  assert.match(signOutMessage(account('a', 'sign_in', 'gmail')), /^Google signed Satchel out of a@x\.test/);
  assert.match(signOutMessage(account('a', 'sign_in', 'graph')), /^Microsoft signed Satchel out/);
  assert.match(signOutMessage(account('a', 'sign_in', 'imap')), /password may have changed/);
});
