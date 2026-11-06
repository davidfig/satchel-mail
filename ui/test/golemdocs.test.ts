// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { GOLEMDOCS_BUILD, GOLEMDOCS_COMMIT, readFromViewer, viewerUrl } from '../src/lib/golemdocs.ts';

describe('readFromViewer', () => {
  test('reads what the viewer sends', () => {
    assert.deepEqual(readFromViewer({ type: 'golemdocs:ready' }), { type: 'golemdocs:ready' });
    assert.deepEqual(readFromViewer({ type: 'golemdocs:opened', name: 'a.docx', format: 'docx', pages: 3 }), { type: 'golemdocs:opened', name: 'a.docx', format: 'docx', pages: 3 });
    assert.deepEqual(readFromViewer({ type: 'golemdocs:error', message: 'Damaged' }), { type: 'golemdocs:error', message: 'Damaged' });
    assert.deepEqual(readFromViewer({ type: 'golemdocs:link', url: 'https://example.com' }), { type: 'golemdocs:link', url: 'https://example.com' });
  });

  test('ignores anything else, and takes nothing on trust', () => {
    assert.equal(readFromViewer(null), null);
    assert.equal(readFromViewer('golemdocs:ready'), null);
    assert.equal(readFromViewer({ type: 'golemdocs:open', bytes: new ArrayBuffer(1), name: 'x' }), null);
    assert.equal(readFromViewer({ type: 'golemdocs:link', url: 42 }), null);
    assert.equal(readFromViewer({ type: 'golemdocs:link', url: 'x'.repeat(9000) }), null);
    assert.deepEqual(readFromViewer({ type: 'golemdocs:error', message: { toString: () => 'x' } }), { type: 'golemdocs:error', message: '' });
    assert.deepEqual(readFromViewer({ type: 'golemdocs:opened', name: 1, format: null, pages: '3' }), { type: 'golemdocs:opened', name: '', format: '', pages: 0 });
  });
});

describe('readFromViewer: keys', () => {
  test('reads a key the viewer passes on, as KeyboardEvent has it', () => {
    const esc = { type: 'golemdocs:key', key: 'Escape', code: 'Escape', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, repeat: false };
    assert.deepEqual(readFromViewer(esc), esc);
    assert.deepEqual(readFromViewer({ type: 'golemdocs:key', key: 'ArrowRight', code: 'ArrowRight', metaKey: true, repeat: true }),
      { type: 'golemdocs:key', key: 'ArrowRight', code: 'ArrowRight', shiftKey: false, ctrlKey: false, altKey: false, metaKey: true, repeat: true });
  });

  test('takes modifiers only when they are really true, and nothing else from the message', () => {
    const m = readFromViewer({ type: 'golemdocs:key', key: 'f', code: 7, ctrlKey: 'yes', metaKey: 1, bubbles: true, target: 'x' });
    assert.deepEqual(m, { type: 'golemdocs:key', key: 'f', code: '', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, repeat: false });
  });

  test('ignores keys that are not keys', () => {
    assert.equal(readFromViewer({ type: 'golemdocs:key' }), null);
    assert.equal(readFromViewer({ type: 'golemdocs:key', key: '' }), null);
    assert.equal(readFromViewer({ type: 'golemdocs:key', key: 'x'.repeat(40) }), null);
  });
});

describe('viewerUrl', () => {
  test('is the pinned build’s page under the host’s base', () => {
    assert.equal(GOLEMDOCS_BUILD, GOLEMDOCS_COMMIT.slice(0, 12));
    assert.equal(viewerUrl('golemdocs://localhost/'), `golemdocs://localhost/${GOLEMDOCS_BUILD}/viewer.html`);
    assert.equal(viewerUrl('http://127.0.0.1:7781/golemdocs'), `http://127.0.0.1:7781/golemdocs/${GOLEMDOCS_BUILD}/viewer.html`);
  });
});
