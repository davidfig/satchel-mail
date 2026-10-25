import type { PartStat } from './calendar';
import type { Address, Id, ISODate } from './common';

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
  /** Can run code or imitate a sign-in page. */
  risky?: boolean;
}

export type WarningKind = 'auth_failed' | 'impersonation' | 'reply_to' | 'deceptive_link' | 'risky_attachment' | 'unknown_inviter';

export interface MessageWarning {
  kind: WarningKind;
  message: string;
}

export interface Unsubscribe {
  url: string | null;
  mailto: string | null;
  one_click: boolean;
}

export type SenderVerdict = 'spam' | 'not_spam';

export interface SenderRule {
  email: string;
  verdict: SenderVerdict;
  created_at: ISODate;
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
  /** May load remote content; when false the frame blocks it too. */
  remote_images_allowed: boolean;
  attachments: Attachment[];
  body_pending: boolean;
  invite?: Invite | null;
  warnings?: MessageWarning[];
  unsubscribe?: Unsubscribe | null;
  sender_verdict?: SenderVerdict | null;
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
  before_id?: Id | null;
  offset?: number | null;
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
