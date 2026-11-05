// Settings → Privacy: remote images, spam sender rules, place lookups, AI drafts.

import { api } from '../../api';
import { h, replace } from '../../lib/dom';
import { relative } from '../../lib/format';
import { errorToast } from '../../lib/ui';
import { store } from '../../state';
import type { SenderRule } from '../../types';
import type { SettingsView } from './index';

export function privacy(view: SettingsView, el: HTMLElement) {
  const s = store.state.settings;
  if (!s) return;
  const aiDrafts = h('input', { type: 'checkbox', checked: s.sync_ai_drafts_to_server, on: { change: () => void view.saveSettings({ ...s, sync_ai_drafts_to_server: aiDrafts.checked }) } });
  const loadAll = h('input', { type: 'checkbox', checked: s.load_remote_images, on: { change: () => void view.saveSettings({ ...s, load_remote_images: loadAll.checked }) } });
  const places = h('input', { type: 'checkbox', checked: s.look_up_places, on: { change: () => void view.saveSettings({ ...s, look_up_places: places.checked }) } });
  // Which service depends on the host: Apple Maps in the Mac app.
  const service = h('span', 'a place service');
  void api.info().then((i) => { if (i.places_service) service.textContent = i.places_service; }, () => {});
  const rules = h('div', h('p.small.muted', 'Loading…'));
  void senderRules(rules);
  replace(el,
    h('h1', 'Privacy'),
    h('h2', 'Remote images'),
    h('p.muted', 'Remote images can tell senders when you open their mail. They are blocked unless you allow a sender.'),
    h('label.row.inline', loadAll, 'Always load remote images, except in spam'),
    s.remote_image_senders.length
      ? h('ul.plain-list', s.remote_image_senders.map((snd) => h('li', snd, h('button.link', { title: 'Stop loading remote images from this sender', on: { click: () => void view.saveSettings({ ...s, remote_image_senders: s.remote_image_senders.filter((x) => x !== snd) }) } }, 'Remove'))))
      : h('p.small.muted', 'No senders allowed.'),
    h('h2', 'Spam'),
    h('p.muted', 'When you mark a message as spam or not spam, Satchel remembers the sender and keeps their mail in the right place, even if your provider’s filter disagrees.'),
    rules,
    h('h2', 'Places'),
    h('p.muted', 'Locations you’ve used in events are suggested as you type one, without anything leaving this device.'),
    h('label.row.inline', places, h('span', 'Also look up places as I type an event’s location. What you type is sent to ', service, '.')),
    h('h2', 'AI drafts'),
    h('label.row.inline', aiDrafts, 'Copy approved AI drafts to the server Drafts folder (so they appear on your phone). They are never sent automatically.'));
}

export async function senderRules(el: HTMLElement) {
  let list: SenderRule[];
  try {
    list = await api.listSenderRules();
  } catch (e) {
    errorToast(e);
    return;
  }
  const remove = async (r: SenderRule) => {
    try {
      await api.setSenderRule(r.email, null);
      void senderRules(el);
    } catch (e) {
      errorToast(e);
    }
  };
  const group = (verdict: SenderRule['verdict'], title: string, none: string, hint: string) => {
    const items = list.filter((r) => r.verdict === verdict);
    return [
      h('h3', title),
      items.length
        ? h('ul.plain-list', items.map((r) => h('li', r.email, h('span.small.muted', relative(r.created_at)), h('button.link', { title: hint, on: { click: () => void remove(r) } }, 'Remove'))))
        : h('p.small.muted', none),
    ];
  };
  replace(el,
    ...group('spam', 'Always spam', 'No senders yet. Use Mark as spam (!) on a message.', 'Stop sending this sender to Spam; your provider’s filter decides again'),
    ...group('not_spam', 'Never spam', 'No senders yet. Use Not spam on a message in Spam.', 'Let your provider’s filter decide for this sender again'));
}
