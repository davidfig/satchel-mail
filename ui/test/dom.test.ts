// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { parseHTML } from 'linkedom';

const { document, Node, Element, Event } = parseHTML('<!doctype html><html><body></body></html>');
Object.assign(globalThis, { document, Node, Element });
const { h, patch } = await import('../src/lib/dom.ts');

describe('patch', () => {
  test('keeps nodes that still match and updates them in place', () => {
    const root = h('nav');
    patch(root, h('button.item', { title: 'Inbox' }, 'Inbox', h('span.count', '3')), h('div.foot', 'Syncing…'));
    const [button, foot] = [...root.children];
    patch(root, h('button.item.active', { title: 'Inbox' }, 'Inbox', h('span.count', '4')), h('div.foot', 'Updated now'));
    assert.equal(root.children[0], button);
    assert.equal(root.children[1], foot);
    assert.equal(button.className, 'item active');
    assert.equal(button.textContent, 'Inbox4');
    assert.equal(foot.textContent, 'Updated now');
  });

  test('replaces nodes whose tag changed and trims extra ones', () => {
    const root = h('nav');
    patch(root, h('span', 'a'), h('span', 'b'), h('span', 'c'));
    const first = root.firstChild;
    patch(root, h('div', 'a'));
    assert.notEqual(root.firstChild, first);
    assert.equal(root.innerHTML, '<div>a</div>');
  });

  test('a kept element runs the new render’s handlers, not the old ones', () => {
    const root = h('nav');
    const calls: string[] = [];
    patch(root, h('button', { on: { click: () => calls.push('old') } }, 'Go'));
    const button = root.firstElementChild!;
    patch(root, h('button', { on: { click: () => calls.push('new') } }, 'Go'));
    assert.equal(root.firstElementChild, button);
    button.dispatchEvent(new Event('click'));
    patch(root, h('button', 'Go'));
    button.dispatchEvent(new Event('click'));
    assert.deepEqual(calls, ['new']);
  });
});
