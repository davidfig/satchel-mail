import type { SyncStatus } from './account';
import type { CalendarAlert } from './calendar';
import type { Address, Id } from './common';

export type CoreEvent =
  | { event: 'accounts_changed' }
  | { event: 'sync_status'; account_id: Id; status: SyncStatus }
  | { event: 'mailboxes_changed'; account_id: Id }
  | { event: 'messages_changed'; account_id: Id }
  | { event: 'calendar_changed'; account_id: Id }
  | { event: 'contacts_changed' }
  | { event: 'favorites_changed' }
  | { event: 'proposals_changed' }
  | { event: 'outbox_changed' }
  | { event: 'op_failed'; account_id: Id; description: string; error: string }
  | { event: 'data_changed' }
  | { event: 'notice'; level: string; message: string }
  | { event: 'new_mail'; account_id: Id; messages: NewMail[] }
  /** Calendar alerts changed: `due` just went off; `closed` stopped showing. */
  | { event: 'alerts_changed'; due: CalendarAlert[]; closed: Id[] };

export interface NewMail {
  message_id: Id;
  thread_id: Id;
  from: Address | null;
  subject: string;
  snippet: string;
}

export interface ApiError {
  code: 'not_found' | 'invalid_input' | 'forbidden' | 'auth' | 'network' | 'conflict' | 'rate_limited' | 'internal';
  message: string;
}
