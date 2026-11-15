"""Turn CHANGELOG.md into the site's changelog page and a release's notes.

Usage:
  python3 scripts/changelog.py html             write site/changelog.html
  python3 scripts/changelog.py check            fail if site/changelog.html is out of date
  python3 scripts/changelog.py notes VERSION    print VERSION's section (fail if there is none)

CHANGELOG.md holds a `## X.Y.Z (YYYY-MM-DD)` section per release, newest first,
each with an optional paragraph and a list of `- ` lines. Only `code`,
**bold** and [links](url) are understood inline; that's all it uses.
"""

import html
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'CHANGELOG.md'
PAGE = ROOT / 'site' / 'changelog.html'
HEADING = re.compile(r'^## (\d+\.\d+\.\d+) \((\d{4}-\d{2}-\d{2})\)$')
MONTHS = 'January February March April May June July August September October November December'.split()


def releases():
    """[(version, date, [lines])] in file order, without HTML comments."""
    text = re.sub(r'<!--.*?-->', '', SOURCE.read_text(encoding='utf-8'), flags=re.S)
    out = []
    for line in text.splitlines():
        m = HEADING.match(line)
        if m:
            out.append((m[1], m[2], []))
        elif line.startswith('## '):
            raise SystemExit(f'CHANGELOG.md: expected "## X.Y.Z (YYYY-MM-DD)", got {line!r}')
        elif out:
            out[-1][2].append(line)
    return out


def inline(s):
    s = html.escape(s, quote=False)
    s = re.sub(r'`([^`]+)`', r'<code>\1</code>', s)
    s = re.sub(r'\*\*([^*]+)\*\*', r'<b>\1</b>', s)
    return re.sub(r'\[([^\]]+)\]\(([^)\s]+)\)', lambda m: f'<a href="{html.escape(m[2])}">{m[1]}</a>', s)


def body_html(lines):
    """Paragraphs and `- ` lists; blank lines separate blocks."""
    out, para, items = [], [], []

    def flush():
        if para:
            out.append(f'<p>{inline(" ".join(para))}</p>')
            para.clear()
        if items:
            out.append('<ul class="plain">' + ''.join(f'<li>{inline(i)}</li>' for i in items) + '</ul>')
            items.clear()

    for line in lines:
        line = line.strip()
        if line.startswith('- '):
            if para:
                flush()
            items.append(line[2:])
        elif not line:
            flush()
        elif items:
            items[-1] += ' ' + line  # a list item wrapped onto the next line
        else:
            para.append(line)
    flush()
    return '\n            '.join(out)


def nice_date(d):
    y, m, day = d.split('-')
    return f'{MONTHS[int(m) - 1]} {int(day)}, {y}'


def page():
    sections = '\n\n'.join(
        f'''        <section class="release" id="v{v}">
          <h2><a href="#v{v}">{v}</a> <time datetime="{d}">{nice_date(d)}</time></h2>
            {body_html(lines)}
        </section>'''
        for v, d, lines in releases()
    )
    return TEMPLATE.replace('{{SECTIONS}}', sections)


def notes(version):
    for v, _, lines in releases():
        if v == version:
            text = '\n'.join(lines).strip()
            if not text:
                break
            return text + '\n\nAlso on the site: https://satchel.email/changelog#v' + v + '\n'
    raise SystemExit(f'CHANGELOG.md has no notes for {version}. Add a "## {version} (YYYY-MM-DD)" section (docs/RELEASING.md).')


# Generated, so edit this, not site/changelog.html. The head and theme script match the other pages
# (site/_headers holds the script's hash).
TEMPLATE = '''<!doctype html>
<!-- Generated from CHANGELOG.md by scripts/changelog.py. Edit that file, then run `python3 scripts/changelog.py html`. -->
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
    <meta name="color-scheme" content="light dark" />
    <title>What's new in satchel.email</title>
    <meta name="description" content="What changed in each release of satchel.email, the local-first email and calendar app." />
    <meta property="og:type" content="website" />
    <meta property="og:site_name" content="satchel.email" />
    <meta property="og:title" content="What's new in satchel.email" />
    <meta property="og:description" content="What changed in each release of satchel.email." />
    <link rel="icon" href="satchel-mark.svg" type="image/svg+xml" />
    <script>
      try {
        var t = localStorage.getItem('satchel.site.theme');
        if (t === 'dark' || t === 'light') document.documentElement.dataset.theme = t;
      } catch (e) {}
    </script>
    <link rel="stylesheet" href="site.css" />
    <script src="site.js" defer></script>
  </head>
  <body>
    <header class="chrome">
      <div class="chrome-in gutter">
        <a class="brand" href="./" aria-label="satchel.email home"><img src="satchel-mark.svg" alt="" width="22" height="22" /><span>satchel.email</span></a>
        <nav class="tabs" aria-label="Pages">
          <a href="mail.html">Mail</a>
          <a href="calendar.html">Calendar</a>
          <a href="ai.html">AI</a>
          <a href="mcp.html">MCP</a>
          <a href="open-source.html">Open source</a>
        </nav>
        <button class="icon-btn" id="theme" type="button" aria-label="Switch light or dark" title="Switch light or dark">
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3.5a6.5 6.5 0 1 0 6.5 6.5A5 5 0 0 1 10 3.5z" /></svg>
        </button>
        <a class="btn primary" href="./#get">Get it</a>
      </div>
    </header>

    <main class="desk gutter">
      <article class="msg changelog">
        <p class="crumb"><a href="./">satchel.email</a> &rsaquo; What's new</p>
        <h1>What's new</h1>
        <p class="lede">What changed in each release, newest first. The app updates itself, so you'll get these within an hour of a release.</p>

{{SECTIONS}}

        <p class="fine">Every release, with its downloads, is also on <a href="https://github.com/davidfig/satchel-mail/releases">GitHub</a>.</p>
      </article>
    </main>

    <footer class="foot gutter">
      <span>satchel.email</span>
      <span class="foot-links">
        <a href="privacy.html">Privacy</a>
        <a href="terms.html">Terms</a>
        <a href="https://github.com/davidfig/satchel-mail">GitHub</a>
        <a href="security.html">Security</a>
        <a href="trademarks.html">Trademarks</a>
        <a href="support.html">Support</a>
        <a href="changelog.html">What&rsquo;s new</a>
      </span>
    </footer>
  </body>
</html>
'''


def main():
    args = sys.argv[1:]
    if args == ['html']:
        PAGE.write_text(page(), encoding='utf-8')
    elif args == ['check']:
        if PAGE.read_text(encoding='utf-8') != page():
            raise SystemExit('site/changelog.html is out of date: run `python3 scripts/changelog.py html` and commit it.')
    elif len(args) == 2 and args[0] == 'notes':
        sys.stdout.reconfigure(encoding='utf-8')  # Windows runners default to cp1252
        sys.stdout.write(notes(args[1].removeprefix('v')))
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    main()
