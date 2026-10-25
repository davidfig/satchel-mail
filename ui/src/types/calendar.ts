import type { Address, Id, ISODate } from './common';

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
