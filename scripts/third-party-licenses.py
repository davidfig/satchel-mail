"""Write the license texts of everything a release ships into one file.

Usage: python3 scripts/third-party-licenses.py OUT [--target TRIPLE]

Covers the crates linked into the `satchel` CLI and the desktop app (normal
dependencies only, for TRIPLE, default the host) and the npm packages the UI
bundles (ui/package.json "dependencies"). Each license text is printed once,
followed by every package it covers. A package that ships no license file
gets the standard text of its declared license (MIT, Zlib or BSD-3-Clause
with its authors, or a pointer to the Apache-2.0 or MPL-2.0 text above); any
still left without one is listed at the end and printed to stderr, for a
release to check by hand.
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROOTS = {'satchel-cli', 'satchel-desktop'}
LICENSE_FILE = re.compile(r'^(licen[cs]e|copying|copyright|notice|unlicense)([-._].*)?$', re.I)

# Standard texts for packages that declare a license but ship no file. The first
# license of an "A OR B" expression found here is the one used.
TEMPLATES = {
    'MIT': '''MIT License

Copyright (c) {authors}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.''',
    'Zlib': '''zlib License

Copyright (c) {authors}

This software is provided 'as-is', without any express or implied
warranty. In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution.''',
    'BSD-3-Clause': '''BSD 3-Clause License

Copyright (c) {authors}

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.''',
}
# Licenses with no copyright line to fill in: point to a copy another package shipped.
FULL_TEXTS = {
    'Apache-2.0': re.compile(r'^\s*Apache License\s+Version 2\.0', re.M),
    'MPL-2.0': re.compile(r'^\s*Mozilla Public License,? Version 2\.0', re.M | re.I),
}


def host_triple():
    out = subprocess.run(['rustc', '-vV'], capture_output=True, text=True, check=True).stdout
    return re.search(r'^host: (.+)$', out, re.M).group(1)


def crates(triple):
    """(name, version, license, authors, dir) for every third-party crate the roots link, for triple."""
    meta = json.loads(subprocess.run(
        ['cargo', 'metadata', '--format-version', '1', '--locked', '--filter-platform', triple],
        cwd=ROOT, capture_output=True, text=True, check=True).stdout)
    packages = {p['id']: p for p in meta['packages']}
    nodes = {n['id']: n for n in meta['resolve']['nodes']}
    todo = [pid for pid, p in packages.items() if p['name'] in ROOTS]
    seen = set(todo)
    while todo:
        for dep in nodes[todo.pop()]['deps']:
            # Build scripts and proc macros run on the build machine; they aren't shipped.
            if any(k['kind'] is None for k in dep['dep_kinds']) and dep['pkg'] not in seen:
                seen.add(dep['pkg'])
                todo.append(dep['pkg'])
    for pid in sorted(seen, key=lambda i: (packages[i]['name'], packages[i]['version'])):
        p = packages[pid]
        if p['source'] is None or 'proc-macro' in p['targets'][0]['kind']:
            continue  # our own workspace crates, or a proc macro
        license = p['license'] or p.get('license_file') or '?'
        authors = ', '.join(re.sub(r'\s*<[^>]*>', '', a) for a in p['authors']) or f"the {p['name']} authors"
        yield p['name'], p['version'], license, authors, Path(p['manifest_path']).parent


def npm_packages():
    ui = ROOT / 'ui'
    for name in sorted(json.loads((ui / 'package.json').read_text()).get('dependencies', {})):
        pkg = ui / 'node_modules' / name
        manifest = json.loads((pkg / 'package.json').read_text())
        author = manifest.get('author') or f'the {name} authors'
        if isinstance(author, dict):
            author = author.get('name', name)
        yield name, manifest['version'], manifest.get('license', '?'), author, pkg


def add(texts, text, user):
    key = hashlib.sha256(text.encode()).hexdigest()
    texts.setdefault(key, (text, []))[1].append(user)


def find(texts, title):
    """The key of a collected text that is the whole license title names."""
    return next((k for k, (text, _) in texts.items() if title.search(text[:200])), None)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('out')
    parser.add_argument('--target', default=None)
    args = parser.parse_args()
    triple = args.target or host_triple()

    texts = {}    # hash -> (text, [package lines])
    unfiled = []  # (line, license, authors) for packages with no license file
    count = 0
    for name, version, license, authors, folder in [*crates(triple), *npm_packages()]:
        count += 1
        line = f'{name} {version} ({license})'
        files = sorted(f for f in folder.iterdir() if f.is_file() and LICENSE_FILE.match(f.name))
        if not files:
            unfiled.append((line, license, authors))
        for f in files:
            add(texts, f.read_text(errors='replace').strip(), f'{line}: {f.name}')

    missing = []
    for line, license, authors in unfiled:
        choices = [c.strip() for c in re.split(r'\s+OR\s+|/', license)]
        template = next((c for c in choices if c in TEMPLATES), None)
        full = next((c for c in choices if c in FULL_TEXTS), None)
        if template:
            add(texts, TEMPLATES[template].format(authors=authors), f'{line}: standard {template} text')
        elif full and (key := find(texts, FULL_TEXTS[full])):
            texts[key][1].append(f'{line}: no file in the package; this is its {full} text')
        else:
            missing.append(line)

    rule = '=' * 78
    with open(args.out, 'w') as out:
        out.write('satchel.email is licensed under the GNU Affero General Public License v3.0\n'
                  '(see LICENSE in the source). It includes the following third-party\n'
                  f'software, each under its own license. {count} packages, built for {triple}.\n')
        for text, users in sorted(texts.values(), key=lambda t: t[1][0]):
            out.write(f'\n{rule}\n' + '\n'.join(users) + f'\n{rule}\n\n{text}\n')
        if missing:
            out.write(f'\n{rule}\nNo license file in the package; declared license shown:\n{rule}\n\n')
            out.write('\n'.join(missing) + '\n')
    for line in missing:
        print(f'no license file: {line}', file=sys.stderr)
    print(f'{args.out}: {count} packages, {len(texts)} license texts', file=sys.stderr)


if __name__ == '__main__':
    main()
