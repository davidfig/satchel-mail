// The event editor: create a new event or change an existing one.

import { api } from '../api';
import { h, icon } from '../lib/dom';
import { parseAddresses } from '../lib/format';
import { errorToast, modal, toast } from '../lib/ui';
import type { CalEvent, Calendar, PartStat } from '../types';

const localTz = Intl.DateTimeFormat().resolvedOptions().timeZone;

function toLocalInput(d: Date) {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** Open the editor for `existing`, or for a new event starting at `at`.
 *  `onSaved` runs after the event is created or updated. */
export function editEvent(calendars: Calendar[], existing: CalEvent | null, onSaved: () => void, at?: Date, allDay = false) {
  const writable = calendars.filter((c) => !c.read_only);
  if (!writable.length) {
    toast('No writable calendars. Add an account with calendar access first.', { kind: 'error' });
    return;
  }
  const start = existing ? new Date(existing.start) : at ?? new Date(Math.ceil(Date.now() / 1_800_000) * 1_800_000);
  const end = existing ? new Date(existing.end) : new Date(start.getTime() + 3_600_000);
  const f = {
    title: h('input', { type: 'text', value: existing?.title ?? '', placeholder: 'Add title', 'aria-label': 'Title' }),
    cal: h('select', { 'aria-label': 'Calendar', disabled: !!existing }, writable.map((c) => h('option', { value: c.id, selected: c.id === existing?.calendar_id }, c.name))),
    allDay: h('input', { type: 'checkbox', checked: existing?.all_day ?? allDay }),
    start: h('input', { type: 'datetime-local', value: toLocalInput(start), 'aria-label': 'Start' }),
    end: h('input', { type: 'datetime-local', value: toLocalInput(end), 'aria-label': 'End' }),
    repeat: h('select', { 'aria-label': 'Repeat' },
      [['', 'Does not repeat'], ['FREQ=DAILY', 'Daily'], ['FREQ=WEEKLY', 'Weekly'], ['FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR', 'Every weekday'], ['FREQ=MONTHLY', 'Monthly'], ['FREQ=YEARLY', 'Yearly']].map(([v, l]) =>
        h('option', { value: v, selected: (existing?.rrule ?? '') === v }, l))),
    location: h('input', { type: 'text', value: existing?.location ?? '', placeholder: 'Location' }),
    attendees: h('input', { type: 'text', value: existing?.attendees.map((a) => a.email).join(', ') ?? '', placeholder: 'Invite people (emails, comma-separated)' }),
    desc: h('textarea', { rows: 4, placeholder: 'Description' }),
  };
  f.desc.value = existing?.description ?? '';
  if (existing?.rrule && ![...f.repeat.options].some((o) => o.value === existing.rrule)) f.repeat.appendChild(h('option', { value: existing.rrule, selected: true }, `Custom (${existing.rrule})`));
  const save = async () => {
    const s = new Date(f.start.value);
    let e = new Date(f.end.value);
    const allDay = f.allDay.checked;
    if (allDay) {
      const sd = new Date(Date.UTC(s.getFullYear(), s.getMonth(), s.getDate()));
      const ed = new Date(Date.UTC(e.getFullYear(), e.getMonth(), e.getDate() + 1));
      s.setTime(sd.getTime());
      e = ed;
    }
    if (!f.title.value.trim()) return toast('Add a title', { kind: 'error' });
    if (e <= s) return toast('End must be after start', { kind: 'error' });
    const prev = existing?.attendees ?? [];
    const attendees = parseAddresses(f.attendees.value).map((a) => prev.find((p) => p.email.toLowerCase() === a.email.toLowerCase()) ?? { email: a.email, name: a.name ?? null, status: 'needs_action' as PartStat });
    const ev: CalEvent = {
      ...(existing ?? { status: 'confirmed', organizer: null, my_status: null, uid: null }),
      id: existing?.id ?? null,
      calendar_id: f.cal.value,
      title: f.title.value.trim(),
      start: s.toISOString(),
      end: e.toISOString(),
      all_day: allDay,
      timezone: allDay ? null : existing?.timezone ?? localTz,
      rrule: f.repeat.value || null,
      location: f.location.value.trim(),
      description: f.desc.value,
      attendees,
    } as CalEvent;
    try {
      if (existing) await api.updateEvent(ev);
      else await api.createEvent(ev);
      m.close();
      onSaved();
      if (attendees.length && !existing) toast('Invitations will be sent by your calendar server');
    } catch (err) {
      errorToast(err);
    }
  };
  const m = modal(existing ? 'Edit event' : 'New event', h('div.event-form',
    f.title,
    h('label.row', 'Calendar', f.cal),
    h('div.row', h('label', 'Starts', f.start), h('label', 'Ends', f.end)),
    h('label.row.inline', f.allDay, 'All day'),
    h('label.row', 'Repeat', f.repeat),
    h('label.row', icon('location', 16), f.location),
    h('label.row', icon('user', 16), f.attendees),
    f.desc,
    h('p.muted.small', `Times are in ${existing?.timezone ?? localTz}.`)), {
    footer: h('div.row.end', h('button', { on: { click: () => m.close() } }, 'Cancel'), h('button.primary', { on: { click: () => void save() } }, existing ? 'Save' : 'Create')),
  });
  f.title.focus();
}
