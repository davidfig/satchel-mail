// Right-click menus for the calendar: events, empty slots and days, and the
// calendar list.

import type { MenuItem } from '../lib/ui';
import type { Calendar, Occurrence, PartStat } from '../types';

/** What the menus can ask the calendar view to do. */
export interface CalendarActions {
  details(o: Occurrence): void;
  editEvent(o: Occurrence): void;
  respond(o: Occurrence, r: PartStat): void;
  deleteEvent(o: Occurrence): void;
  newEvent(at: Date, allDay?: boolean): void;
  openDay(d: Date): void;
  /** Set each calendar's visibility to `visible(c)`. */
  setVisible(visible: (c: Calendar) => boolean): void;
}

const RSVP: [PartStat, string][] = [['accepted', 'Going'], ['tentative', 'Maybe'], ['declined', 'Not going']];

export function eventMenu(act: CalendarActions, o: Occurrence, cal: Calendar | undefined): (MenuItem | '-')[] {
  const e = o.event;
  const writable = !!cal && !cal.read_only;
  return [
    { label: 'Open', icon: 'calendar', run: () => act.details(o) },
    ...(writable ? [{ label: 'Edit…', icon: 'draft', run: () => act.editEvent(o) }] : []),
    ...(e.my_status
      ? ['-' as const, ...RSVP.map(([r, label]) => ({ label, icon: e.my_status === r ? 'check' : undefined, disabled: e.my_status === r, run: () => act.respond(o, r) }))]
      : []),
    ...(writable ? ['-' as const, { label: e.rrule ? 'Delete all occurrences' : 'Delete', icon: 'trash', danger: true, run: () => act.deleteEvent(o) }] : []),
  ];
}

/** Menu for an empty spot in the grid. `at` carries the time when the click
 *  was in a timed column; otherwise the day is all that matters. */
export function slotMenu(act: CalendarActions, at: Date, timed: boolean, dayView: boolean): (MenuItem | '-')[] {
  const time = at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  const day = at.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
  return [
    timed
      ? { label: `New event at ${time}`, icon: 'plus', run: () => act.newEvent(at) }
      : { label: `New event on ${day}`, icon: 'plus', run: () => act.newEvent(new Date(at.getFullYear(), at.getMonth(), at.getDate(), 9)) },
    { label: 'New all-day event', icon: 'calendar', run: () => act.newEvent(at, true) },
    ...(dayView ? [] : ['-' as const, { label: `Open ${day}`, icon: 'chevronRight', run: () => act.openDay(at) }]),
  ];
}

export function calendarMenu(act: CalendarActions, c: Calendar, all: Calendar[]): (MenuItem | '-')[] {
  const others = all.filter((x) => x.id !== c.id);
  return [
    { label: c.visible ? 'Hide' : 'Show', icon: c.visible ? 'close' : 'check', run: () => act.setVisible((x) => (x.id === c.id ? !c.visible : x.visible)) },
    ...(others.length
      ? [
          { label: 'Show only this calendar', icon: 'calendar', disabled: c.visible && others.every((x) => !x.visible), run: () => act.setVisible((x) => x.id === c.id) },
          { label: 'Show all calendars', icon: 'check', disabled: all.every((x) => x.visible), run: () => act.setVisible(() => true) },
        ]
      : []),
  ];
}
