// Settings → Accounts → Send-as addresses: other addresses an account sends
// mail as, typed in by hand (0072).

import { api } from '../../api';
import { h, icon } from '../../lib/dom';
import { modal, toast } from '../../lib/ui';
import { store } from '../../state';
import type { Account, Credential, SendAs, Security } from '../../types';
import { field, secSelect } from './form';

/** Microsoft 365 can't send as other addresses through Graph (0072). */
export const canSendAs = (a: Account) => a.kind !== 'graph';

const HELP: Record<string, string> = {
  gmail: 'Gmail sends these only if each one is also set up and verified under “Send mail as” in Gmail’s settings (Accounts and Import). If it isn’t, Gmail sends from your Gmail address and Satchel warns you.',
  imap: 'Each address can use this account’s SMTP server, or its own (for example a domain that only allows its own server to send for it).',
  jmap: 'Each address must also be a sending identity in your provider’s settings.',
};

interface Row {
  el: HTMLElement;
  read: () => { sendAs: SendAs; credential?: Credential };
}

export function editSendAs(a: Account) {
  const rows: Row[] = [];
  const list = h('div.send-as-list');
  const status = h('p.small');

  const addRow = (s?: SendAs) => {
    const email = h('input', { type: 'email', value: s?.email ?? '', placeholder: 'hello@example.com', 'aria-label': 'Address' });
    const name = h('input', { type: 'text', value: s?.display_name ?? '', placeholder: a.display_name || 'Name (optional)', 'aria-label': 'Name' });
    const own = h('input', { type: 'checkbox', checked: !!s?.smtp });
    const host = h('input', { type: 'text', value: s?.smtp?.host ?? '', placeholder: 'smtp.example.com' });
    const port = h('input', { type: 'number', value: String(s?.smtp?.port ?? 465) });
    const sec = secSelect(s?.smtp?.security ?? 'tls');
    const user = h('input', { type: 'text', value: s?.smtp?.username ?? '', autocomplete: 'off' });
    const pass = h('input', { type: 'password', autocomplete: 'new-password', placeholder: s?.smtp ? 'Saved; type to change' : 'Password or token' });
    const server = h('div.send-as-server',
      h('div.row', field('SMTP server', host), field('Port', port), field('Security', sec)),
      h('div.row', field('Username', user), field('Password', pass)));
    server.hidden = !own.checked;
    own.addEventListener('change', () => (server.hidden = !own.checked));
    const el = h('div.send-as-row',
      h('div.row', field('Address', email), field('Name', name),
        h('button.icon-btn', {
          title: 'Remove this address',
          'aria-label': 'Remove this address',
          on: { click: () => { rows.splice(rows.indexOf(row), 1); el.remove(); } },
        }, icon('trash', 16))),
      a.kind === 'imap' && [h('label.row.inline', own, 'Send through its own SMTP server'), server]);
    const row: Row = {
      el,
      read: () => {
        const smtp = a.kind === 'imap' && own.checked
          ? { host: host.value.trim(), port: Number(port.value), security: sec.value as Security, username: user.value.trim() }
          : null;
        return {
          sendAs: { email: email.value.trim(), display_name: name.value.trim(), smtp },
          credential: smtp && pass.value ? { type: 'password', password: pass.value } : undefined,
        };
      },
    };
    rows.push(row);
    list.append(el);
    if (!s) email.focus();
  };
  for (const s of a.send_as ?? []) addRow(s);

  const save = async () => {
    if (btn.disabled) return;
    const read = rows.map((r) => r.read()).filter((r) => r.sendAs.email);
    const creds: Record<string, Credential> = {};
    for (const r of read) if (r.credential) creds[r.sendAs.email.toLowerCase()] = r.credential;
    status.className = 'small';
    status.textContent = Object.keys(creds).length || read.some((r) => r.sendAs.smtp) ? 'Checking…' : 'Saving…';
    btn.disabled = true;
    try {
      await api.setSendAs(a.id, read.map((r) => r.sendAs), creds);
      m.close();
      toast('Send-as addresses saved.', { quiet: true });
      await store.refreshAccounts();
    } catch (e) {
      status.className = 'small error';
      status.textContent = e instanceof Error ? e.message : String(e);
    } finally {
      btn.disabled = false;
    }
  };
  const btn = h('button.primary', { title: 'Check any new SMTP servers and save these addresses', on: { click: () => void save() } }, 'Save');
  const m = modal(`Send-as addresses for ${a.email}`, h('div.form',
    h('p.small.muted', 'Addresses you can choose as From when writing from this account. Replies go out from the address the message was sent to. ', HELP[a.kind] ?? ''),
    list,
    h('div', h('button', { title: 'Add another address to send as', on: { click: () => addRow() } }, icon('plus', 14), 'Add address')),
    status), {
    wide: true,
    footer: h('div.row.end', h('button', { title: 'Close without saving', on: { click: () => m.close() } }, 'Cancel'), btn),
  });
}
