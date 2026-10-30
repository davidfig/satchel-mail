import type { CalEvent, PartStat } from './calendar';
import type { Id, ISODate } from './common';
import type { Contact } from './contacts';
import type { Draft } from './mail';

export type ProposalAction =
  | { type: 'draft'; draft: Draft }
  | { type: 'labels'; message_ids: Id[]; add: Id[]; remove: Id[] }
  | { type: 'archive'; message_ids: Id[] }
  | { type: 'move'; message_ids: Id[]; mailbox_id: Id }
  | { type: 'mark_read'; message_ids: Id[]; seen: boolean }
  | { type: 'flag'; message_ids: Id[]; flagged: boolean }
  | { type: 'create_event'; event: CalEvent }
  | { type: 'update_event'; event: CalEvent }
  | { type: 'cancel_event'; event_id: Id }
  | { type: 'respond_to_invite'; event_id: Id; response: PartStat }
  | { type: 'save_contact'; contact: Contact }
  /** The first contact is kept; the others are merged into it. */
  | { type: 'merge_contacts'; contact_ids: Id[] };

export type ProposalStatus = 'pending' | 'approved' | 'applied' | 'rejected' | 'stale' | 'failed';

export interface Proposal {
  id: Id;
  agent_id: Id;
  agent_name: string;
  created_at: ISODate;
  status: ProposalStatus;
  action: ProposalAction;
  rationale: string;
  source_thread_ids: Id[];
  warnings: string[];
  review_note: string | null;
  decided_at: ISODate | null;
  summary: string;
}

export interface AgentPolicy {
  account_ids: Id[] | null;
  since_days: number | null;
  allow_attachments: boolean;
  allow_calendar: boolean;
  allow_contacts: boolean;
  max_proposals_per_hour: number;
}

export interface Agent {
  id: Id;
  name: string;
  created_at: ISODate;
  last_used: ISODate | null;
  policy: AgentPolicy;
  revoked: boolean;
  /** Built into Satchel (the AI folder), not created by the user. */
  builtin: boolean;
}

export interface MirrorSettings {
  enabled: boolean;
  root: string | null;
  account_ids: Id[] | null;
  since_days: number | null;
  include_eml: boolean;
  include_attachments: boolean;
  accept_proposals: boolean;
  include_calendar: boolean;
  include_contacts: boolean;
}

export interface Settings {
  mirror: MirrorSettings;
  remote_image_senders: string[];
  sync_ai_drafts_to_server: boolean;
  theme: 'system' | 'light' | 'dark' | string;
  initial_sync_days: number;
  /** Days of mail downloaded in the background (null: all mail). */
  download_days: number | null;
  /** Most disk each account's downloaded mail may use, in MB (null: no limit). */
  storage_limit_mb: number | null;
  poll_interval_secs: number;
  notify_new_mail: boolean;
}

export interface AuditEntry {
  id: number;
  at: ISODate;
  actor: string;
  action: string;
  detail: Record<string, unknown>;
}

export interface OutboxItem {
  id: Id;
  account_id: Id;
  kind: string;
  status: string;
  attempts: number;
  last_error: string | null;
  created_at: ISODate;
}
