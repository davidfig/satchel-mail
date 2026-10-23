// Mirrors crates/api/src/lib.rs. Keep in sync when the schema changes.

export type Id = string;
export type ISODate = string;

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
      allow_invalid_certs?: boolean;
    }
  | { kind: 'jmap'; email: string; display_name: string; username: string; session_url: string; caldav_url?: string | null; allow_invalid_certs?: boolean }
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
}

export type MailboxRole = 'inbox' | 'sent' | 'drafts' | 'trash' | 'junk' | 'archive' | 'all' | 'flagged' | 'important' | 'custom';

export interface Mailbox {
  id: Id;
  account_id: Id;
  name: string;
  path: string;
  role: MailboxRole;
  is_label: boolean;
  unread: number;
  total: number;
}

export interface Address {
  name?: string | null;
  email: string;
}

export interface Flags {
  seen: boolean;
  flagged: boolean;
  answered: boolean;
  draft: boolean;
}

export interface ThreadSummary {
  thread_id: Id;
  account_id: Id;
  subject: string;
  participants: Address[];
  last_date: ISODate;
  message_count: number;
  unread_count: number;
  flagged: boolean;
  has_attachments: boolean;
  snippet: string;
  mailbox_ids: Id[];
  latest_message_id: Id;
}

export interface Attachment {
  id: string;
  filename: string;
  mime_type: string;
  size: number;
  inline: boolean;
  content_id: string | null;
}

export interface MessageDetail {
  id: Id;
  account_id: Id;
  thread_id: Id;
  subject: string;
  from: Address | null;
  to: Address[];
  date: ISODate;
  snippet: string;
  flags: Flags;
  mailbox_ids: Id[];
  has_attachments: boolean;
  cc: Address[];
  bcc: Address[];
  reply_to: Address[];
  message_id_header: string | null;
  in_reply_to: string | null;
  references: string[];
  text_body: string;
  html_body: string | null;
  remote_images_blocked: boolean;
  attachments: Attachment[];
  body_pending: boolean;
  invite?: Invite | null;
}

export interface Invite {
  uid: string;
  method: string;
  title: string;
  start: ISODate;
  end: ISODate;
  all_day: boolean;
  location: string;
  organizer: Address | null;
  my_status: PartStat | null;
  event_id: Id | null;
}

export interface Thread {
  thread_id: Id;
  account_id: Id;
  subject: string;
  messages: MessageDetail[];
}

export interface ThreadQuery {
  mailbox_id?: Id | null;
  role?: MailboxRole | null;
  account_id?: Id | null;
  search?: string | null;
  limit?: number | null;
  before?: ISODate | null;
}

export interface OutgoingAttachment {
  filename: string;
  mime_type: string;
  data_base64: string;
}

export interface Draft {
  id?: Id | null;
  account_id: Id;
  to: Address[];
  cc: Address[];
  bcc: Address[];
  subject: string;
  body_text: string;
  body_html?: string | null;
  in_reply_to?: Id | null;
  forward_of?: Id | null;
  attachments: OutgoingAttachment[];
}

export type ReplyMode = 'reply' | 'reply_all' | 'forward';

export interface Calendar {
  id: Id;
  account_id: Id;
  name: string;
  color: string;
  read_only: boolean;
  visible: boolean;
}

export type EventStatus = 'confirmed' | 'tentative' | 'cancelled';
export type PartStat = 'needs_action' | 'accepted' | 'declined' | 'tentative';

export interface Attendee {
  email: string;
  name?: string | null;
  status: PartStat;
  optional?: boolean;
}

export interface CalEvent {
  id?: Id | null;
  calendar_id: Id;
  uid?: string | null;
  title: string;
  description: string;
  location: string;
  start: ISODate;
  end: ISODate;
  all_day: boolean;
  timezone?: string | null;
  rrule?: string | null;
  attendees: Attendee[];
  organizer?: Address | null;
  status: EventStatus;
  my_status?: PartStat | null;
}

export interface Occurrence {
  event: CalEvent;
  start: ISODate;
  end: ISODate;
  color: string;
}

export interface FreeSlot {
  start: ISODate;
  end: ISODate;
}

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
  | { type: 'respond_to_invite'; event_id: Id; response: PartStat };

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
  max_proposals_per_hour: number;
}

export interface Agent {
  id: Id;
  name: string;
  created_at: ISODate;
  last_used: ISODate | null;
  policy: AgentPolicy;
  revoked: boolean;
}

export interface MirrorSettings {
  enabled: boolean;
  root: string | null;
  account_ids: Id[] | null;
  since_days: number | null;
  include_eml: boolean;
  include_attachments: boolean;
}

export interface Settings {
  mirror: MirrorSettings;
  remote_image_senders: string[];
  sync_ai_drafts_to_server: boolean;
  theme: 'system' | 'light' | 'dark' | string;
  initial_sync_days: number;
  poll_interval_secs: number;
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

export type CoreEvent =
  | { event: 'accounts_changed' }
  | { event: 'sync_status'; account_id: Id; status: SyncStatus }
  | { event: 'mailboxes_changed'; account_id: Id }
  | { event: 'messages_changed'; account_id: Id }
  | { event: 'calendar_changed'; account_id: Id }
  | { event: 'proposals_changed' }
  | { event: 'outbox_changed' }
  | { event: 'op_failed'; account_id: Id; description: string; error: string }
  | { event: 'data_changed' }
  | { event: 'notice'; level: string; message: string };

export interface ApiError {
  code: 'not_found' | 'invalid_input' | 'forbidden' | 'auth' | 'network' | 'conflict' | 'rate_limited' | 'internal';
  message: string;
}
