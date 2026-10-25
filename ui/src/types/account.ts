import type { Id, ISODate } from './common';

export type ProviderKind = 'imap' | 'gmail' | 'graph' | 'jmap';
export type Security = 'tls' | 'start_tls' | 'none';

export type AccountConfig =
  | {
      kind: 'imap';
      email: string;
      display_name: string;
      username: string;
      imap_host: string;
      imap_port: number;
      imap_security: Security;
      smtp_host: string;
      smtp_port: number;
      smtp_security: Security;
      caldav_url?: string | null;
      carddav_url?: string | null;
      allow_invalid_certs?: boolean;
    }
  | { kind: 'jmap'; email: string; display_name: string; username: string; session_url: string; caldav_url?: string | null; carddav_url?: string | null; allow_invalid_certs?: boolean }
  | { kind: 'gmail'; email: string; display_name: string }
  | { kind: 'graph'; email: string; display_name: string };

export type Credential =
  | { type: 'password'; password: string }
  | { type: 'o_auth'; access_token: string; refresh_token?: string | null; expires_at?: number | null };

export type SyncState = 'idle' | 'syncing' | 'error' | 'offline';
export interface SyncStatus {
  state: SyncState;
  last_success: ISODate | null;
  error: string | null;
  detail: string | null;
}

export interface Account {
  id: Id;
  kind: ProviderKind;
  email: string;
  display_name: string;
  config: AccountConfig;
  sync: SyncStatus;
  color: string;
  /** Signed in before contacts access was requested; contacts sync after signing in again. */
  contacts_need_reconnect?: boolean;
}
