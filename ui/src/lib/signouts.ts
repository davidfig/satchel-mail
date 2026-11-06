// Accounts the provider signed out: what to say about them, and which are new.

import type { Account } from '../types';

export function isSignedOut(a: Account): boolean {
  return a.sync.state === 'sign_in';
}

/** What happened to a signed-out account, for the banner and the notification. */
export function signOutMessage(a: Account): string {
  if (a.kind === 'gmail') return `Google signed Satchel out of ${a.email}, so this account has stopped syncing.`;
  if (a.kind === 'graph') return `Microsoft signed Satchel out of ${a.email}, so this account has stopped syncing.`;
  return `Satchel can’t sign in to ${a.email}, so this account has stopped syncing. Its password may have changed.`;
}

/** Accounts signed out since the last call. `seen` tracks them: an account
 *  that signs back in leaves it, so a later sign-out counts as new again. */
export function newlySignedOut(accounts: Account[], seen: Set<string>): Account[] {
  const out = accounts.filter(isSignedOut);
  const ids = new Set(out.map((a) => a.id));
  for (const id of seen) if (!ids.has(id)) seen.delete(id);
  const fresh = out.filter((a) => !seen.has(a.id));
  for (const a of fresh) seen.add(a.id);
  return fresh;
}
