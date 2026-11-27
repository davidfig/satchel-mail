// Run with `npm test`.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { unwrap } from '../src/lib/unwrap.ts';

const mail = `Mr. Figglesworth,

How have things been? Hard to believe it's been almost a decade since our
last adventures. I'm hoping this email address is still valid and this
actually reaches you.

I've got a few questions for you. Firstly, are you still in Taiwan? I ask
because HJ and I will be visiting in about a month's time, and I figured
I'd see if you were around.

-C`.split('\n');

test('joins the lines a 72-column wrap broke', () => {
  assert.deepEqual(unwrap(mail), [
    'Mr. Figglesworth,',
    '',
    "How have things been? Hard to believe it's been almost a decade since our last adventures. I'm hoping this email address is still valid and this actually reaches you.",
    '',
    "I've got a few questions for you. Firstly, are you still in Taiwan? I ask because HJ and I will be visiting in about a month's time, and I figured I'd see if you were around.",
    '',
    '-C',
  ]);
});

test('keeps breaks the sender typed', () => {
  const lines = [
    'Here is what we need from the store before the party on Saturday night:',
    '- milk',
    '- a cake big enough for everybody who said they would come along',
    '',
    'Thanks,',
    'Sam',
    '--',
    'Sam Smith, Head of Things at a company with a fairly long name here',
    '> quoted text that was wrapped by the other client at seventy-two cols',
    '> and continues here',
  ];
  assert.deepEqual(unwrap(lines), lines);
});

test('leaves text alone that was never hard-wrapped', () => {
  const short = ['Roses are red,', 'violets are blue'];
  assert.deepEqual(unwrap(short), short);
  const long = ['A'.repeat(60) + ' ' + 'b '.repeat(40), 'next line'];
  assert.deepEqual(unwrap(long), long);
});
