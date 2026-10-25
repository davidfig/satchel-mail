import type { Address, Id, ISODate } from './common';

export interface AddressBook {
  id: Id;
  account_id: Id;
  name: string;
  read_only: boolean;
}

export interface ContactEmail {
  email: string;
  label?: string | null;
}

export interface ContactPhone {
  number: string;
  label?: string | null;
}

export type ContactOrigin = 'auto' | 'local' | 'remote';

export interface ContactSource {
  address_book_id: Id;
  account_id: Id;
  name: string;
}

export interface Contact {
  id?: Id | null;
  name: string;
  emails: ContactEmail[];
  phones: ContactPhone[];
  organization: string;
  title: string;
  /** YYYY-MM-DD, or --MM-DD without a year. */
  birthday?: string | null;
  notes: string;
  // Read-only, filled in by the core.
  origin?: ContactOrigin;
  sources?: ContactSource[];
  message_count?: number;
  last_contacted?: ISODate | null;
  version?: number;
}

/** Everything exchanged with one person, across their addresses. */
export interface PersonSummary {
  /** The contact these addresses belong to; its other addresses are included. */
  contact_id: Id | null;
  name: string | null;
  addresses: Address[];
  received: number;
  sent: number;
  threads: number;
  attachments: number;
  first_date: string | null;
  last_date: string | null;
  events: PersonEvent[];
}

export interface PersonEvent {
  event_id: Id;
  title: string;
  start: string;
  recurring: boolean;
}
