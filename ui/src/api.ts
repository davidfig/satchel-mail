// Typed wrappers over the command schema.

import { transport } from './transport';
import type {
  Account,
  Address,
  AccountConfig,
  Agent,
  AgentPolicy,
  AuditEntry,
  CalEvent,
  Calendar,
  Credential,
  Draft,
  FreeSlot,
  Id,
  Mailbox,
  MessageDetail,
  Occurrence,
  OutboxItem,
  PartStat,
  Proposal,
  ProposalAction,
  ProposalStatus,
  ProviderKind,
  ReplyMode,
  Settings,
  Thread,
  ThreadQuery,
  ThreadSummary,
} from './types';

const call = <T>(cmd: string, args?: unknown) => transport.invoke<T>(cmd, args);

export const api = {
  info: () => call<{ version: string; mode: string; data_dir: string; mirror_root: string }>('info'),

  listAccounts: () => call<Account[]>('list_accounts'),
  testAccount: (config: AccountConfig, credential: Credential) => call<{ ok: boolean }>('test_account', { config, credential }),
  addAccount: (config: AccountConfig, credential: Credential) => call<Account>('add_account', { config, credential }),
  removeAccount: (account_id: Id) => call('remove_account', { account_id }),
  updateAccount: (account_id: Id, config: AccountConfig, credential: Credential | null) => call<Account>('update_account', { account_id, config, credential }),
  startOAuth: (provider: ProviderKind) => call<{ auth_url: string }>('start_o_auth', { provider }),
  syncNow: (account_id: Id | null = null) => call('sync_now', { account_id }),

  listMailboxes: (account_id: Id | null = null) => call<Mailbox[]>('list_mailboxes', { account_id }),
  listThreads: (query: ThreadQuery) => call<ThreadSummary[]>('list_threads', { query }),
  countThreads: (query: ThreadQuery) => call<{ total: number }>('count_threads', { query }),
  getThread: (thread_id: Id) => call<Thread>('get_thread', { thread_id }),
  getMessage: (message_id: Id) => call<MessageDetail>('get_message', { message_id }),
  getRaw: (message_id: Id) => call<{ raw: string }>('get_raw_message', { message_id }),
  getAttachment: (message_id: Id, attachment_id: string) =>
    call<{ filename: string; mime_type: string; data_base64: string }>('get_attachment', { message_id, attachment_id }),
  loadRemoteImages: (message_id: Id, always_for_sender: boolean) => call<MessageDetail>('load_remote_images', { message_id, always_for_sender }),

  setFlags: (message_ids: Id[], seen: boolean | null, flagged: boolean | null) => call('set_flags', { message_ids, seen, flagged }),
  move: (message_ids: Id[], mailbox_id: Id) => call('move', { message_ids, mailbox_id }),
  archive: (message_ids: Id[]) => call('archive', { message_ids }),
  trash: (message_ids: Id[]) => call('trash', { message_ids }),
  setLabels: (message_ids: Id[], add: Id[], remove: Id[]) => call('set_labels', { message_ids, add, remove }),
  saveDraft: (draft: Draft) => call<{ draft_id: Id }>('save_draft', { draft }),
  deleteDraft: (draft_id: Id) => call('delete_draft', { draft_id }),
  listDrafts: () => call<Draft[]>('list_drafts'),
  send: (draft: Draft) => call<{ op_id: Id }>('send', { draft }),
  composeReply: (message_id: Id, mode: ReplyMode) => call<Draft>('compose_reply', { message_id, mode }),
  suggestAddresses: (prefix: string) => call<Address[]>('suggest_addresses', { prefix }),

  listCalendars: () => call<Calendar[]>('list_calendars'),
  setCalendarVisible: (calendar_id: Id, visible: boolean) => call('set_calendar_visible', { calendar_id, visible }),
  listEvents: (from: string, to: string, calendar_ids: Id[] | null = null) => call<Occurrence[]>('list_events', { from, to, calendar_ids }),
  getEvent: (event_id: Id) => call<CalEvent>('get_event', { event_id }),
  createEvent: (event: CalEvent) => call<CalEvent>('create_event', { event }),
  updateEvent: (event: CalEvent) => call<CalEvent>('update_event', { event }),
  deleteEvent: (event_id: Id) => call('delete_event', { event_id }),
  respondToInvite: (event_id: Id, response: PartStat) => call('respond_to_invite', { event_id, response }),
  respondToEmailInvite: (message_id: Id, response: PartStat) => call<{ ok: boolean; via: string }>('respond_to_email_invite', { message_id, response }),
  findFreeTime: (from: string, to: string, duration_minutes: number, timezone: string) =>
    call<FreeSlot[]>('find_free_time', { from, to, duration_minutes, day_start_hour: 9, day_end_hour: 17, timezone }),

  listProposals: (status: ProposalStatus | null = null) => call<Proposal[]>('list_proposals', { status }),
  approveProposal: (proposal_id: Id, edited: ProposalAction | null = null) =>
    call<{ ok: boolean; draft_id?: Id; event_id?: Id }>('approve_proposal', { proposal_id, edited }),
  rejectProposal: (proposal_id: Id, note: string | null) => call('reject_proposal', { proposal_id, note }),

  listAgents: () => call<Agent[]>('list_agents'),
  createAgent: (name: string, policy: AgentPolicy) => call<{ agent: Agent; token: string }>('create_agent', { name, policy }),
  updateAgent: (agent_id: Id, name: string | null, policy: AgentPolicy | null) => call<Agent>('update_agent', { agent_id, name, policy }),
  revokeAgent: (agent_id: Id) => call('revoke_agent', { agent_id }),

  getSettings: () => call<Settings>('get_settings'),
  updateSettings: (settings: Settings) => call<Settings>('update_settings', { settings }),
  listAudit: (limit = 200) => call<AuditEntry[]>('list_audit', { limit }),
  listOutbox: () => call<OutboxItem[]>('list_outbox'),
  retryOutbox: (id: Id) => call('retry_outbox', { id }),
};
