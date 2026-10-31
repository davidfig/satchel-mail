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
  /** Reminders that show on screen. Always set when read; null on a write keeps the stored ones (0056). */
  alerts?: Alert[] | null;
}

/** A reminder of an event, relative to its start. */
export interface Alert {
  /** Minutes before the start; negative is after. All-day events count from midnight on their first day. */
  minutes_before: number;
}

/** An alert that has gone off and hasn't been dismissed. */
export interface CalendarAlert {
  /** `{event_id}:{occurrence start, ms}:{minutes_before}`: known before it goes off. */
  id: Id;
  event_id: Id;
  calendar_id: Id;
  title: string;
  location: string;
  /** The occurrence the alert is for. */
  start: ISODate;
  end: ISODate;
  all_day: boolean;
  color: string;
  minutes_before: number;
  /** When it goes off: the alert's time, or (if snoozed) the snooze's end. */
  due: ISODate;
  snoozed: boolean;
}

export interface Occurrence {
  event: CalEvent;
  start: ISODate;
  end: ISODate;
  color: string;
}

export type PlaceSource = 'history' | 'open_street_map' | 'apple_maps';

/** A suggestion for an event's location. */
export interface PlaceSuggestion {
  /** What goes in the location field. */
  text: string;
  label: string;
  detail: string | null;
  source: PlaceSource;
}

export interface FreeSlot {
  start: ISODate;
  end: ISODate;
}
