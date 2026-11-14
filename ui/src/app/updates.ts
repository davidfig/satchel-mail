// Offers to restart into a new version once the desktop app has downloaded it (0063).

import { transport } from '../transport';
import { errorToast, toast, type ToastHandle } from '../lib/ui';

function offer(version: string) {
  toast(`satchel.email ${version} is ready to install`, {
    sticky: true,
    action: { label: 'Restart to update', run: () => void transport.host('install_update').catch(errorToast) },
  });
}

/** The toast of a check the user asked for, while it runs. */
let checking: ToastHandle | null = null;

export function installUpdates() {
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
