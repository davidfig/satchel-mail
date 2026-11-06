// Offers to restart into a new version once the desktop app has downloaded it (0063).

import { transport } from '../transport';
import { errorToast, toast } from '../lib/ui';

function offer(version: string) {
  toast(`satchel.email ${version} is ready to install`, {
    sticky: true,
    action: { label: 'Restart to update', run: () => void transport.host('install_update').catch(errorToast) },
  });
}

export function installUpdates() {
  let offered = '';
  transport.onUpdateReady((version) => {
    if (version === offered) return;
    offered = version;
    offer(version);
  });
}

let checking = false;

/** Help > Check for Updates: looks now, and says what it found. A version
 *  already offered and dismissed is offered again, since the user asked. */
export async function checkForUpdates() {
  if (checking) return;
  checking = true;
  toast('Checking for updates…');
  try {
    const { current, ready } = await transport.host<{ current: string; ready: string | null }>('check_for_updates');
    if (ready) offer(ready);
    else toast(`You have the latest version (${current}).`, { kind: 'success' });
  } catch (e) {
    toast(`Couldn’t check for updates: ${e instanceof Error ? e.message : String(e)}`, { kind: 'error', ms: 7000 });
  } finally {
    checking = false;
  }
}
