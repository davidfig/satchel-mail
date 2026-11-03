// Run with `npm test`.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { blobType, documentName, parseCsv, previewKind } from '../src/lib/files.ts';

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

  test('sends documents to golemdocs', () => {
    assert.equal(previewKind(file('report.docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document')), 'document');
    assert.equal(previewKind(file('Old.DOC')), 'document');
    assert.equal(previewKind(file('minutes.odt')), 'document');
    assert.equal(previewKind(file('letter.rtf', 'text/rtf')), 'document');
    assert.equal(previewKind(file('book.epub')), 'document');
    assert.equal(previewKind(file('README.md', 'text/plain')), 'document');
    assert.equal(previewKind(file('paper.tex', 'text/x-tex')), 'document');
    assert.equal(previewKind(file('analysis.ipynb', 'application/json')), 'document');
    assert.equal(previewKind(file('report', 'application/vnd.oasis.opendocument.text')), 'document');
  });

  test('shows web pages through golemdocs only, even though they are risky to open', () => {
    // The core flags these risky: a browser would run them. golemdocs reads them into its model.
    assert.equal(previewKind(file('page.html', 'text/html', true)), 'document');
    assert.equal(previewKind(file('Saved.MHTML', 'application/x-mimearchive', true)), 'document');
    assert.equal(previewKind(file('logo.svg', 'image/svg+xml', true)), 'document');
    assert.equal(previewKind(file('logo.svgz', 'application/octet-stream', true)), 'document');
  });

  test('never shows other risky files, whatever they claim to be', () => {
    assert.equal(previewKind(file('invoice.pdf', 'application/pdf', true)), null);
    assert.equal(previewKind(file('invoice.pdf', 'text/html', true)), null);
    assert.equal(previewKind(file('page', 'text/html', true)), null);
    assert.equal(previewKind(file('macro.docm', 'application/vnd.ms-word.document.macroEnabled.12', true)), null);
    assert.equal(previewKind(file('setup.exe', 'image/svg+xml', true)), null);
    assert.equal(previewKind(file('page.html.', 'text/html', true)), null);
  });

  test('has nothing for files it can only open', () => {
    assert.equal(previewKind(file('budget.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')), null);
    assert.equal(previewKind(file('archive.zip')), null);
  });

  test('leaves PDFs to the platform viewer', () => {
    assert.equal(previewKind(file('scan.pdf', 'application/pdf')), 'pdf');
  });
});

describe('documentName', () => {
  test('keeps a name that says what the file is', () => {
    assert.equal(documentName(file('Report.docx', 'application/octet-stream')), 'Report.docx');
    assert.equal(documentName(file('notes.md', 'text/plain')), 'notes.md');
  });

  test('adds the extension a document named without one needs', () => {
    assert.equal(documentName(file('Report', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document')), 'Report.docx');
    assert.equal(documentName(file('minutes.final', 'application/vnd.oasis.opendocument.text; name=x')), 'minutes.final.odt');
    assert.equal(documentName(file('', 'text/markdown')), 'attachment.md');
  });
});

describe('blobType', () => {
  test('is only ever a type the webview shows, not one it runs', () => {
    assert.equal(blobType('pdf', file('x.pdf', 'text/html')), 'application/pdf');
    assert.equal(blobType('text', file('x.txt', 'text/html')), 'text/plain');
    assert.equal(blobType('image', file('x.jpg')), 'image/jpeg');
    assert.equal(blobType('image', file('x.tif')), 'image/tiff');
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
