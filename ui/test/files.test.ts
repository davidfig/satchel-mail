// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { blobType, parseCsv, previewKind } from '../src/lib/files.ts';

const file = (filename: string, mime_type = 'application/octet-stream', risky = false) => ({ filename, mime_type, risky });

describe('previewKind', () => {
  test('goes by type, then by extension', () => {
    assert.equal(previewKind(file('scan', 'application/pdf')), 'pdf');
    assert.equal(previewKind(file('Scan.PDF')), 'pdf');
    assert.equal(previewKind(file('photo', 'image/jpeg; name=photo')), 'image');
    assert.equal(previewKind(file('IMG_0001.HEIC')), 'image');
    assert.equal(previewKind(file('memo.m4a')), 'audio');
    assert.equal(previewKind(file('clip.mov')), 'video');
    assert.equal(previewKind(file('data.csv', 'text/plain')), 'csv');
    assert.equal(previewKind(file('notes', 'text/plain')), 'text');
    assert.equal(previewKind(file('invite.ics', 'application/ics')), 'text');
  });

  test('never shows HTML or risky files, whatever they claim to be', () => {
    assert.equal(previewKind(file('page.html', 'text/plain')), null);
    assert.equal(previewKind(file('page', 'text/html')), null);
    assert.equal(previewKind(file('invoice.pdf', 'application/pdf', true)), null);
  });

  test('has nothing for files it can only open', () => {
    assert.equal(previewKind(file('report.docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document')), null);
    assert.equal(previewKind(file('archive.zip')), null);
  });
});

describe('blobType', () => {
  test('is only ever a type the webview shows, not one it runs', () => {
    assert.equal(blobType('pdf', file('x.pdf', 'text/html')), 'application/pdf');
    assert.equal(blobType('text', file('x.txt', 'text/html')), 'text/plain');
    assert.equal(blobType('image', file('x.jpg')), 'image/jpeg');
    assert.equal(blobType('image', file('x.svg')), 'image/svg+xml');
    assert.equal(blobType('video', file('x.mov')), 'video/quicktime');
  });
});

describe('parseCsv', () => {
  test('reads quoted fields, escaped quotes and line breaks inside quotes', () => {
    assert.deepEqual(parseCsv('a,b\r\n"1,5","say ""hi"""\n"two\nlines",x', ','), [['a', 'b'], ['1,5', 'say "hi"'], ['two\nlines', 'x']]);
  });

  test('keeps empty fields and ignores a final newline', () => {
    assert.deepEqual(parseCsv('a,,c\n', ','), [['a', '', 'c']]);
    assert.deepEqual(parseCsv('a\tb', '\t'), [['a', 'b']]);
  });
});
