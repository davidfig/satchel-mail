// The `#follow=` page (web only). Links in HTML mail point here (see
// `routeLinks`), so a click opens this in a new tab even where no listener
// caught it. The address is untrusted, since any website can link here: it
// always shows where the link goes and waits for a click. It never reads or
// changes the session.

import { h, replace } from '../lib/dom';
import { linkScheme } from '../lib/thread';
import { installTooltips } from '../lib/tooltip';
import { confirmLink } from '../views/mail/message-body';

export async function followPage(url: string, cautious: boolean) {
  installTooltips();
  document.title = 'Open link? · satchel.email';
  const page = h('div.fatal');
  document.body.append(page);
  if (linkScheme(url) !== 'web') {
    replace(page, h('h1', 'Satchel didn’t open this link'), h('p', 'Only web and email links from messages can be opened.'), h('p.link-address', url));
    return;
  }
  if (await confirmLink(url, cautious, true)) {
    location.replace(url);
    return;
  }
  window.close();
  // Browsers only let a page close tabs it opened; say so if this one stays.
  replace(page,
    h('h1', 'Link not opened'),
    h('p', 'You can close this tab.'),
    h('button', { title: 'Open satchel.email in this tab', on: { click: () => location.replace('/') } }, 'Go to Satchel'));
}
