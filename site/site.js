// The marketing site's two bits of behavior: the light/dark switch, and the AI Proposals demo, where approving a
// proposal shows its effect in the mailbox beside it.

(() => {
  const root = document.documentElement;
  const dark = () => root.dataset.theme === 'dark' || (root.dataset.theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.getElementById('theme')?.addEventListener('click', () => {
    root.dataset.theme = dark() ? 'light' : 'dark';
    try { localStorage.setItem('satchel.site.theme', root.dataset.theme); } catch (e) {}
  });

  // Downloads: the visitor's build up front, the others behind "Other systems". Macs default to Apple silicon;
  // Chromium browsers can say when it's an Intel Mac.
  const ua = navigator.userAgent;
  const os = /Windows/.test(ua) ? 'windows'
    : /Mac OS X|Macintosh/.test(ua) && !/iPhone|iPad/.test(ua) && navigator.maxTouchPoints < 2 ? 'mac'
    : /Linux/.test(ua) && !/Android/.test(ua) ? 'linux' : '';
  const builds = [...document.querySelectorAll('#getAll .dl')];
  const pick = (arch) => builds.find((b) => b.dataset.os === os && (!arch || b.dataset.arch === arch));
  const download = document.getElementById('download');
  const others = document.getElementById('getOthers');
  const all = document.getElementById('getAll');

  const show = (build) => {
    const btn = document.getElementById('getBtn');
    btn.href = build.href;
    btn.textContent = build.dataset.label;
    document.getElementById('getNote').textContent = build.querySelector('span').textContent;
    document.getElementById('getMine').hidden = false;
    const alt = document.getElementById('getAlt');
    const other = os === 'mac' && builds.find((b) => b.dataset.os === 'mac' && b !== build);
    alt.hidden = !other && os !== 'linux';
    if (other) {
      alt.replaceChildren(other.dataset.arch === 'x64' ? 'Intel Mac? ' : 'Apple silicon Mac? ', Object.assign(document.createElement('a'), { href: other.href, textContent: 'Download that one' }));
    } else if (os === 'linux') {
      alt.replaceChildren('Prefer a package? ', Object.assign(document.createElement('a'), { href: all.querySelector('a[href$=".deb"]').href, textContent: 'Download the .deb' }));
    }
    all.hidden = true;
    others.hidden = false;
  };

  if (os && pick()) {
    if (download) download.textContent = `Download for ${os === 'mac' ? 'Mac' : os === 'windows' ? 'Windows' : 'Linux'}`;
    show(pick(os === 'mac' ? 'arm64' : ''));
    navigator.userAgentData?.getHighEntropyValues?.(['architecture'])
      .then((v) => { if (os === 'mac' && v.architecture === 'x86') show(pick('x64')); })
      .catch(() => {});
  }
  others?.addEventListener('click', () => {
    all.hidden = !all.hidden;
    others.setAttribute('aria-expanded', String(!all.hidden));
    others.textContent = all.hidden ? 'Other systems' : 'Hide other systems';
  });

  // The hero switches between the mail and calendar views, from the switch above it or its own sidebar.
  const hero = document.getElementById('hero');
  if (hero) {
    const tabs = [...document.querySelectorAll('.seg [data-view]')];
    const show = (view) => {
      hero.dataset.view = view;
      hero.setAttribute('aria-label', view === 'cal'
        ? 'satchel.email showing a week of events from three calendars'
        : 'satchel.email showing a unified inbox and a conversation with an invitation in it');
      tabs.forEach((t) => t.setAttribute('aria-selected', String(t.dataset.view === view)));
    };
    tabs.forEach((t) => t.addEventListener('click', () => show(t.dataset.view)));
    hero.querySelectorAll('[data-go]').forEach((r) => r.addEventListener('click', () => show(r.dataset.go)));
  }

  const demo = document.getElementById('demo');
  if (!demo) return;
  const props = [...demo.querySelectorAll('.prop')];
  const archived = [...demo.querySelectorAll('[data-archive]')];
  const bob = demo.querySelector('[data-thread="bob"]');
  const dentist = demo.querySelector('[data-thread="dentist"]');
  const ev = document.getElementById('evNew');
  const inboxCount = document.getElementById('inboxCount');
  const propCount = document.getElementById('propCount');
  const empty = document.getElementById('reviewEmpty');

  const flash = (row) => {
    row.classList.add('flash');
    setTimeout(() => row.classList.remove('flash'), 900);
  };
  const effects = {
    reply: () => { bob.classList.remove('unread'); bob.classList.add('replied'); flash(bob); return 'Sent to Bob.'; },
    archive: () => { archived.forEach((r, i) => setTimeout(() => r.classList.add('gone'), i * 120)); return 'Archived 3 threads.'; },
    event: () => { ev.classList.add('in'); dentist.classList.remove('unread'); flash(dentist); return 'Added to Thursday.'; },
  };

  const update = () => {
    const open = props.filter((p) => !p.classList.contains('approved') && !p.classList.contains('rejected')).length;
    propCount.textContent = String(open);
    propCount.hidden = open === 0;
    inboxCount.textContent = String(demo.querySelectorAll('.list .row:not(.gone)').length);
    empty.hidden = open > 0;
  };

  for (const p of props) {
    const done = p.querySelector('.prop-done');
    p.querySelector('[data-approve]').addEventListener('click', () => {
      done.textContent = effects[p.dataset.prop]();
      p.classList.add('approved');
      setTimeout(update, 450);
    });
    p.querySelector('[data-reject]').addEventListener('click', () => {
      done.textContent = 'Rejected. Claude can read why.';
      p.classList.add('rejected');
      update();
    });
  }

  document.getElementById('resetDemo').addEventListener('click', () => {
    props.forEach((p) => p.classList.remove('approved', 'rejected'));
    archived.forEach((r) => r.classList.remove('gone'));
    bob.classList.remove('replied');
    bob.classList.add('unread');
    dentist.classList.add('unread');
    ev.classList.remove('in');
    update();
  });
})();
