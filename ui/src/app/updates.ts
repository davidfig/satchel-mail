// Offers to restart into a new version once the desktop app has downloaded it (0063), and says
// once that it installed. Both toasts link to that version's notes on the site's What's New page (0082).

import { transport } from '../transport';
import { errorToast, toast, type ToastHandle } from '../lib/ui';
import { openExternal } from '../views/mail/message-body';
import { HELP_LINKS } from './help';

/** The version this window last ran, to tell that an update installed. */
const LAST_VERSION_KEY = 'satchel.lastVersion';

/** Opens `version`'s section of the changelog; the toast stays up. */
function whatsNew(version: string) {
  return {
    label: 'What’s new',
    title: `What changed in ${version}`,
    keep: true,
    run: () => openExternal(`${HELP_LINKS['whats-new']}#v${version}`),
  };
}

function offer(version: string) {
  toast(`satchel.email ${version} is ready to install`, {
    sticky: true,
    action: [
      { label: 'Restart to update', run: () => void transport.host('install_update').catch(errorToast) },
      whatsNew(version),
    ],
  });
}

/** After a restart into a new version, says so once, with its notes a click away. Not on a first run. */
async function announceUpdate() {
  const version = await transport.host<string>('app_version');
  let last: string | null = null;
  try {
    last = localStorage.getItem(LAST_VERSION_KEY);
    localStorage.setItem(LAST_VERSION_KEY, version);
  } catch {
    return;
  }
  if (last && last !== version) toast(`Updated to satchel.email ${version}`, { kind: 'success', ms: 12000, action: whatsNew(version) });
}

/** The toast of a check the user asked for, while it runs. */
let checking: ToastHandle | null = null;

export function installUpdates() {
  if (transport.kind === 'tauri') void announceUpdate().catch(() => {});
  // Only a check the user is waiting on shows the download; the timer's stays quiet.
  transport.onUpdateProgress(({ version, percent }) => {
    checking?.setMessage(`Downloading satchel.email ${version}…${percent === null ? '' : ` ${percent}%`}`);
  });
  let offered = '';
  transport.onUpdateReady((version) => {
    if (version === offered) return;
    offered = version;
    offer(version);
  });
}

/** Help > Check for Updates: looks now, and says what it's doing and what
 *  it found. A newer version downloads first (tens of MB), so the toast
 *  stays up with its progress until then. A version already offered and
 *  dismissed is offered again, since the user asked. */
export async function checkForUpdates() {
  if (checking) return;
  const shown = toast('Checking for updates…', { persist: true, fixed: true });
  checking = shown;
  try {
    const { current, ready } = await transport.host<{ current: string; ready: string | null }>('check_for_updates');
    shown.dismiss();
    if (ready) offer(ready);
    else toast(`You’re on the latest version (${current}).`, { kind: 'success' });
  } catch (e) {
    shown.dismiss();
    toast(`Couldn’t check for updates: ${e instanceof Error ? e.message : String(e)}`, { kind: 'error', ms: 7000 });
  } finally {
    checking = null;
  }
}
