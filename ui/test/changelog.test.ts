// Run with `npm test`.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, test } from 'node:test';
import { niceDate, parseBlocks, parseChangelog, parseInline } from '../src/app/changelog-notes.ts';

describe('changelog', () => {
  test('reads every release in CHANGELOG.md, newest first, each with notes', () => {
    const source = readFileSync(new URL('../../CHANGELOG.md', import.meta.url), 'utf8');
    const releases = parseChangelog(source);
    assert.equal(releases.length, source.match(/^## \d+\.\d+\.\d+ \(/gm)?.length);
    assert.equal(releases.at(-1)?.version, '0.1.0');
    for (const r of releases) assert.ok(r.blocks.length, `${r.version} has no notes`);
  });

  test('skips comments, and splits paragraphs from lists', () => {
    const [r] = parseChangelog('# Changelog\n<!-- ## 9.9.9 (2026-01-01) -->\n## 1.2.3 (2026-11-15)\n\nIntro\ngoes on.\n\n- one\n  wrapped\n- two\n');
    assert.equal(r.version, '1.2.3');
    assert.deepEqual(r.blocks, [{ kind: 'p', text: 'Intro goes on.' }, { kind: 'ul', items: ['one wrapped', 'two'] }]);
  });

  test('feed notes without a heading parse as a body', () => {
    assert.deepEqual(parseBlocks('- a\r\n- b'), [{ kind: 'ul', items: ['a', 'b'] }]);
  });

  test('inline code, bold and http links; other links stay text', () => {
    assert.deepEqual(parseInline('Open `~/Satchel` **now**: [site](https://satchel.email) or [x](javascript:alert(1))'), [
      { text: 'Open ' }, { text: '~/Satchel', code: true }, { text: ' ' }, { text: 'now', bold: true }, { text: ': ' },
      { text: 'site', href: 'https://satchel.email' }, { text: ' or ' }, { text: 'x' }, { text: ')' },
    ]);
  });

  test('dates read as on the site', () => {
    assert.equal(niceDate('2026-11-05'), 'November 5, 2026');
  });
});
