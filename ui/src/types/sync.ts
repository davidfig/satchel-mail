// Mirrors crates/api/src/sync.rs: settings sync between computers (0080).

import type { AccountConfig } from './account';

export interface SyncDevice {
  id: string;
  name: string;
  /** When that computer last wrote its file. */
  last_seen: string;
}

/** An account set up on another computer and not here yet. */
export interface OfferedAccount {
  config: AccountConfig;
  color: string;
}

export interface FolderSync {
  enabled: boolean;
  folder: string | null;
  device_id: string | null;
  device_name: string | null;
  devices: SyncDevice[];
  last_pass: string | null;
  error: string | null;
  offered_accounts: OfferedAccount[];
}
