// Accounts that were signed out: the banner above every view, and the way back in.

import { h, icon } from '../lib/dom';
import { relative } from '../lib/format';
import { isSignedOut, signOutMessage } from '../lib/signouts';
import type { Account } from '../types';
import { changePassword, isOAuth, reconnect } from '../views/settings/accounts';

/** Sign in again in the browser, or for a password account, enter a new password. */
export function signInAgain(a: Account) {
  if (isOAuth(a)) void reconnect(a);
  else changePassword(a);
}

/** One banner per signed-out account. They stay until it's signed back in or removed. */
export function signInBanners(accounts: Account[]): HTMLElement[] {
  return accounts.filter(isSignedOut).map((a) =>
    h('div.banner.warn', { role: 'status' },
      icon('warning', 16),
      h('span.grow', `${signOutMessage(a)} ${a.sync.last_success ? `Last synced ${relative(a.sync.last_success)}.` : ''}`),
      isOAuth(a)
        ? h('button', { title: 'Sign in again in your browser to resume syncing', on: { click: () => signInAgain(a) } }, 'Sign in again')
        : h('button', { title: 'Enter this account’s new password to resume syncing', on: { click: () => signInAgain(a) } }, 'Update password')));
}
