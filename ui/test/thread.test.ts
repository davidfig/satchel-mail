// Run with `npm test`. Samples are shaped like the core's sanitizer output: only the
// attributes it keeps (class, style, dir, div id, blockquote type) survive.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { DOMParser } from 'linkedom';
import { looksDesigned, planFold, stripHtmlQuotes, trimUrl } from '../src/lib/thread.ts';

globalThis.DOMParser = DOMParser as unknown as typeof globalThis.DOMParser;

/** Asserts the reply text survives, the history is gone, and the original is kept for "show quoted text". */
function expectStripped(html: string, kept: string[], gone: string[]) {
  const r = stripHtmlQuotes(html);
  for (const k of kept) assert.ok(r.html.includes(k), `expected to keep ${JSON.stringify(k)} in ${r.html}`);
  for (const g of gone) assert.ok(!r.html.includes(g), `expected to drop ${JSON.stringify(g)} from ${r.html}`);
  assert.equal(r.quoted, html);
}

describe('stripHtmlQuotes', () => {
  test('Gmail', () => {
    expectStripped(
      '<div dir="ltr">Sounds good, see you Monday.</div><br><div class="gmail_quote"><div dir="ltr" class="gmail_attr">On Mon, Oct 20, 2026 at 9:26 AM Bob &lt;bob@satchel.test&gt; wrote:<br></div><blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex"><div dir="ltr">Can we kick off on Monday?</div></blockquote></div>',
      ['Sounds good, see you Monday.'],
      ['wrote:', 'Can we kick off', 'gmail_quote'],
    );
  });

  test('Apple Mail, attribution inside its own cite block', () => {
    expectStripped(
      '<div dir="auto">Works for me.</div><div dir="auto"><br><blockquote type="cite">On Oct 20, 2026, at 9:26 AM, Bob &lt;bob@satchel.test&gt; wrote:<br><br></blockquote></div><blockquote type="cite"><div dir="ltr">Can we kick off on Monday?</div></blockquote>',
      ['Works for me.'],
      ['wrote:', 'Can we kick off', 'blockquote'],
    );
  });

  test('Apple Mail, attribution line just before the quote', () => {
    expectStripped(
      '<div>Works for me.</div><div>On Oct 20, 2026, at 9:26 AM, Bob wrote:</div><blockquote type="cite"><div>Can we kick off on Monday?</div></blockquote>',
      ['Works for me.'],
      ['wrote:', 'Can we kick off'],
    );
  });

  test('Outlook on the web', () => {
    expectStripped(
      '<div class="elementToProof" style="font-family:Aptos">Approved.</div><div id="appendonsend"></div><hr style="display:inline-block;width:98%"><div id="divRplyFwdMsg" dir="ltr"><font face="Calibri, sans-serif" style="font-size:11pt" color="#000000"><b>From:</b> Carol &lt;carol@satchel.test&gt;<br><b>Sent:</b> Monday, October 20, 2026 2:26 PM<br><b>To:</b> Alice<br><b>Subject:</b> Q3 budget</font><div>&nbsp;</div></div><div><div dir="ltr">Any thoughts? Need to lock it by Friday.</div></div>',
      ['Approved.'],
      ['From:', 'Any thoughts', '<hr'],
    );
  });

  test('Outlook desktop, no markers but a From/Sent/Subject header', () => {
    expectStripped(
      '<div class="WordSection1"><p class="MsoNormal">Let\'s keep travel as-is.</p><p class="MsoNormal">&nbsp;</p><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0in 0in 0in"><p class="MsoNormal"><b>From:</b> Dan &lt;dan@satchel.test&gt;<br><b>Sent:</b> Tuesday, October 21, 2026 11:02 AM<br><b>To:</b> Bob<br><b>Subject:</b> RE: Q3 budget</p></div><p class="MsoNormal">&nbsp;</p><p class="MsoNormal">Travel is already 15% under last year.</p></div>',
      ["Let's keep travel as-is.", 'WordSection1'],
      ['From:', 'Travel is already'],
    );
  });

  test('Outlook desktop, Date: instead of Sent:', () => {
    expectStripped(
      '<p>Thanks!</p><div><p><b>From:</b> Dan<br><b>Date:</b> Tuesday, October 21, 2026<br><b>Subject:</b> Q3</p></div><p>Old text</p>',
      ['Thanks!'],
      ['From:', 'Old text'],
    );
  });

  test('Thunderbird', () => {
    expectStripped(
      '<p>Sure, Thursday works.</p><div class="moz-cite-prefix">On 10/20/26 9:26 AM, Bob wrote:<br></div><blockquote type="cite"><p>Thursday?</p></blockquote>',
      ['Sure, Thursday works.'],
      ['wrote:', 'Thursday?'],
    );
  });

  test('Yahoo, by class', () => {
    expectStripped(
      '<div class="ydp1"><div>Yes please.</div></div><div class="yahoo_quoted"><div>On Monday, October 20, 2026, Bob wrote:</div><blockquote><div>Lunch?</div></blockquote></div>',
      ['Yes please.'],
      ['wrote:', 'Lunch?'],
    );
  });

  test('Yahoo, by id', () => {
    expectStripped(
      '<div>Yes please.</div><div id="yahoo_quoted_8812"><div>On Monday, Bob wrote:</div><blockquote><div>Lunch?</div></blockquote></div>',
      ['Yes please.'],
      ['wrote:', 'Lunch?'],
    );
  });

  test('cuts at the first marker when the history nests several clients', () => {
    expectStripped(
      '<div>Agreed.</div><div class="gmail_quote">On Tue, Dan wrote:<blockquote class="gmail_quote"><div id="divRplyFwdMsg"><b>From:</b> Bob</div><div>Older</div></blockquote></div>',
      ['Agreed.'],
      ['Dan wrote', 'Older', 'From:'],
    );
  });

  test('leaves mail without quoted history alone', () => {
    const html = '<div>Hi Alice,</div><blockquote><p>A plain blockquote is content, not history.</p></blockquote><p>Bob</p>';
    assert.deepEqual(stripHtmlQuotes(html), { html, quoted: null });
  });

  test('keeps the original when nothing but history would be left', () => {
    const html = '<br><div class="gmail_quote"><div class="gmail_attr">---------- Forwarded message ---------</div><div>Original text</div></div>';
    assert.deepEqual(stripHtmlQuotes(html), { html, quoted: null });
  });

  test('keeps an image-only reply', () => {
    const r = stripHtmlQuotes('<div><img src="data:image/png;base64,AAAA" alt="chart"></div><div class="gmail_quote">On Mon, Bob wrote:<blockquote>Send the chart?</blockquote></div>');
    assert.ok(r.html.includes('<img'));
    assert.ok(!r.html.includes('Send the chart'));
  });

  test('known limitation: text written below the quote is folded with it', () => {
    const r = stripHtmlQuotes('<div>See inline.</div><div class="gmail_quote"><blockquote>Question 1?</blockquote><div>Answer 1.</div></div><div>Thanks</div>');
    assert.ok(r.html.includes('See inline.'));
    assert.ok(!r.html.includes('Answer 1.'));
    assert.ok(!r.html.includes('Thanks'));
  });
});

describe('looksDesigned', () => {
  test('tables and several images keep formatting', () => {
    assert.equal(looksDesigned('<table><tr><td>Receipt</td></tr></table>'), true);
    assert.equal(looksDesigned('<p>Hi</p><img src="a"><img src="b">'), true);
  });
  test('plain replies, even with a signature logo, become bubbles', () => {
    assert.equal(looksDesigned('<div>Thanks!</div>'), false);
    assert.equal(looksDesigned('<div>Thanks!</div><div><img src="logo"></div>'), false);
  });
});

describe('planFold', () => {
  const me = 'alice@satchel.test';
  const mine = (a: { email: string } | null | undefined) => a?.email === me;
  /** 'r' read from someone else, 'n' new from someone else, 'm' mine. */
  const thread = (pattern: string) =>
    [...pattern].map((c) => ({ flags: { seen: c !== 'n', flagged: false, answered: false, draft: false }, from: { email: c === 'm' ? me : 'bob@satchel.test', name: null } }));
  const plan = (pattern: string) => planFold(thread(pattern) as never, mine);

  test('short threads never fold', () => {
    assert.deepEqual(plan(''), { firstNew: -1, fold: null });
    assert.deepEqual(plan('r'), { firstNew: -1, fold: null });
    assert.deepEqual(plan('rmrn'), { firstNew: 3, fold: null });
    assert.deepEqual(plan('rrrrr'), { firstNew: -1, fold: null });
  });

  test('all read: first message, fold, latest three', () => {
    assert.deepEqual(plan('rmrmrmrmrm'), { firstNew: -1, fold: { start: 1, end: 7 } });
  });

  test('new messages: fold stops one message before the first new one', () => {
    assert.deepEqual(plan('rrrrrrrrnn'), { firstNew: 8, fold: { start: 1, end: 7 } });
    assert.deepEqual(plan('rrrrnrrrrr'), { firstNew: 4, fold: { start: 1, end: 3 } });
  });

  test('a fold of one message is not worth it', () => {
    assert.deepEqual(plan('rrrnrrrrrr'), { firstNew: 3, fold: null });
  });

  test('everything new: nothing folds', () => {
    assert.deepEqual(plan('nnnnnnnnnn'), { firstNew: 0, fold: null });
  });

  test('my own unread messages are not "new"', () => {
    const msgs = thread('rrrrrrrrrr');
    msgs[5].flags.seen = false;
    msgs[5].from.email = me;
    assert.deepEqual(planFold(msgs as never, mine), { firstNew: -1, fold: { start: 1, end: 7 } });
  });
});

describe('trimUrl', () => {
  test('drops trailing punctuation and an unopened closing bracket', () => {
    assert.deepEqual(trimUrl('https://x.test/notes).'), ['https://x.test/notes', ').']);
    assert.deepEqual(trimUrl('https://x.test/a,'), ['https://x.test/a', ',']);
    assert.deepEqual(trimUrl('https://x.test/a]'), ['https://x.test/a', ']']);
  });
  test('keeps brackets that belong to the URL', () => {
    assert.deepEqual(trimUrl('https://en.wikipedia.org/wiki/Satchel_(bag)'), ['https://en.wikipedia.org/wiki/Satchel_(bag)', '']);
    assert.deepEqual(trimUrl('https://en.wikipedia.org/wiki/Satchel_(bag)).'), ['https://en.wikipedia.org/wiki/Satchel_(bag)', ').']);
    assert.deepEqual(trimUrl('https://x.test/?q=a'), ['https://x.test/?q=a', '']);
  });
});
