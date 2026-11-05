// Offers to restart into a new version once the desktop app has downloaded it (0063).

import { transport } from '../transport';
import { errorToast, toast } from '../lib/ui';

export function installUpdates() {
  let offered = '';
  transport.onUpdateReady((version) => {
    if (version === offered) return;
    offered = version;
    toast(`satchel.email ${version} is ready to install`, {
      sticky: true,
      action: { label: 'Restart to update', run: () => void transport.host('install_update').catch(errorToast) },
    });
  });
}
