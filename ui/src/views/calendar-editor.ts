// The event editor: create a new event or change an existing one.

import { api } from '../api';
import { combobox, type Suggestion } from '../lib/combobox';
import { h, icon, listen } from '../lib/dom';
import { parseAddresses } from '../lib/format';
import { fuzzy } from '../lib/fuzzy';
import { allZones, fromWall, localZone, offsetLabel, offsetMinutes, toWall, zoneCity, zoneLongName } from '../lib/tz';
import { confirmDialog, errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { Alert, CalEvent, Calendar, PartStat, PlaceSource } from '../types';
import { alertsPicker } from './calendar-alerts';
import { repeatPicker } from './calendar-repeat';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** Open the editor for `existing`, or for a new event starting at `at`.
 *  `onSaved` runs after the event is created or updated. */
export function editEvent(calendars: Calendar[], existing: CalEvent | null, onSaved: () => void, at?: Date, allDay = false) {
  const writable = calendars.filter((c) => !c.read_only);
  if (!writable.length) {
    toast('No writable calendars. Add an account with calendar access first.', { kind: 'error' });
    return;
  }
  const isAllDay = existing?.all_day ?? allDay;
  // Moving an event with guests would cancel and re-invite them (0049).
  const hasGuests = !!existing?.attendees.length;
  let zone = existing?.timezone ?? localZone;
  const start = existing ? new Date(existing.start) : at ?? new Date(Math.ceil(Date.now() / (HOUR / 2)) * (HOUR / 2));
  const end = existing ? new Date(existing.end) : new Date(start.getTime() + (isAllDay ? DAY : HOUR));
  // All-day events are UTC midnights with an exclusive end (a new one from a
  // click is a local midnight). Their times start at 9–10 in case the user
  // unticks "All day".
  const s0 = existing?.all_day ? toWall(start, 'UTC') : toWall(start, zone);
  const e0 = existing?.all_day ? toWall(new Date(end.getTime() - DAY), 'UTC') : isAllDay ? s0 : toWall(end, zone);
  if (isAllDay) [s0.time, e0.time] = ['09:00', '10:00'];

  const f = {
    title: h('input.ev-title', { type: 'text', value: existing?.title ?? '', placeholder: 'Add title', 'aria-label': 'Title' }),
    startDate: h('input', { type: 'date', value: s0.date, 'aria-label': 'Start date' }),
    startTime: h('input', { type: 'time', value: s0.time, 'aria-label': 'Start time' }),
    endDate: h('input', { type: 'date', value: e0.date, 'aria-label': 'End date' }),
    endTime: h('input', { type: 'time', value: e0.time, 'aria-label': 'End time' }),
    allDay: h('input', { type: 'checkbox', checked: isAllDay }),
    zone: h('input', { type: 'text', 'aria-label': 'Time zone' }),
    location: h('input', { type: 'text', value: existing?.location ?? '', placeholder: 'Add location', 'aria-label': 'Location' }),
    attendees: h('input', { type: 'text', value: existing?.attendees.map((a) => a.email).join(', ') ?? '', placeholder: 'Invite people by email', 'aria-label': 'Guests' }),
    cal: h('select', { 'aria-label': 'Calendar', disabled: hasGuests }, calendarOptions(writable, existing?.calendar_id)),
    desc: h('textarea', { rows: 3, placeholder: 'Add description', 'aria-label': 'Description' }),
  };
  f.desc.value = existing?.description ?? '';
  // New timed events start with the default alert from Settings.
  const defaultAlerts = (all: boolean): Alert[] => {
    const m = store.state.settings?.default_alert_minutes;
    return m == null || all ? [] : [{ minutes_before: m }];
  };
  const alerts = alertsPicker(existing ? existing.alerts ?? [] : defaultAlerts(isAllDay), () => f.allDay.checked, defaultAlerts);
  const repeat = repeatPicker(existing?.rrule ?? null, () => ({ date: f.startDate.value, time: f.startTime.value, zone, allDay: f.allDay.checked }));

  const zoneRow = h('div.ev-zone', f.zone, icon('chevronDown', 16));
  const hint = h('div.ev-hint.muted.small');
  const times = [f.startTime, f.endTime];

  const read = () => {
    if (f.allDay.checked) {
      return { s: fromWall(f.startDate.value, '00:00', 'UTC'), e: new Date(fromWall(f.endDate.value, '00:00', 'UTC').getTime() + DAY) };
    }
    return { s: fromWall(f.startDate.value, f.startTime.value, zone), e: fromWall(f.endDate.value, f.endTime.value, zone) };
  };
  const write = (input: { date: HTMLInputElement; time: HTMLInputElement }, at: Date) => {
    const w = toWall(at, zone);
    input.date.value = w.date;
    input.time.value = w.time;
  };
  /** The zone and its offset on the event's day ("Berlin (GMT+1)"). */
  const zoneLabel = () => {
    const s = read().s;
    return `${zoneCity(zone)} (${offsetLabel(zone, isNaN(s.getTime()) ? new Date() : s)})`;
  };
  const refresh = () => {
    repeat.refresh();
    alerts.refresh();
    const all = f.allDay.checked;
    for (const t of times) t.hidden = all;
    zoneRow.hidden = all;
    f.zone.value = zoneLabel();
    // Say when the event falls in the user's own time, if that's elsewhere.
    const { s, e } = read();
    const away = !all && !isNaN(s.getTime()) && !isNaN(e.getTime()) && offsetMinutes(zone, s) !== offsetMinutes(localZone, s);
    hint.hidden = !away;
    if (away) {
      const day = (d: Date) => d.toLocaleDateString([], { weekday: 'short' });
      const time = (d: Date) => d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
      const sameDay = s.toDateString() === new Date(e.getTime() - 1).toDateString();
      hint.textContent = `${day(s)} ${time(s)} – ${sameDay ? '' : `${day(e)} `}${time(e)} your time (${zoneCity(localZone)})`;
    }
  };

  // Moving the start moves the end with it, keeping the length.
  let lastStart = read().s;
  const startMoved = () => {
    const { s, e } = read();
    if (isNaN(s.getTime())) return;
    const length = e.getTime() - lastStart.getTime();
    if (!isNaN(length) && length > 0) {
      if (f.allDay.checked) f.endDate.value = toWall(new Date(s.getTime() + length - DAY), 'UTC').date;
      else write({ date: f.endDate, time: f.endTime }, new Date(s.getTime() + length));
    }
    lastStart = s;
    refresh();
  };
  listen(f.startDate, 'change', startMoved);
  listen(f.startTime, 'change', startMoved);
  listen(f.endDate, 'change', () => { lastStart = read().s; refresh(); });
  listen(f.endTime, 'change', () => { lastStart = read().s; refresh(); });
  listen(f.allDay, 'change', () => { lastStart = read().s; refresh(); });

  // The zone keeps the wall-clock times as typed: "9 AM in Berlin".
  const locationRow = locationPicker(f.location);
  zonePicker(f.zone, () => zone, zoneLabel, (z) => {
    zone = z;
    lastStart = read().s;
    refresh();
  });
  refresh();

  const save = async () => {
    const all = f.allDay.checked;
    const { s, e } = read();
    if (!f.title.value.trim()) return toast('Add a title', { kind: 'error' });
    if (isNaN(s.getTime()) || isNaN(e.getTime())) return toast('Enter a start and end', { kind: 'error' });
    if (e <= s) return toast('End must be after start', { kind: 'error' });
    const problem = repeat.problem();
    if (problem) return toast(problem, { kind: 'error' });
    const prev = existing?.attendees ?? [];
    const attendees = parseAddresses(f.attendees.value).map((a) => prev.find((p) => p.email.toLowerCase() === a.email.toLowerCase()) ?? { email: a.email, name: a.name ?? null, status: 'needs_action' as PartStat });
    const ev: CalEvent = {
      ...(existing ?? { status: 'confirmed', organizer: null, my_status: null, uid: null }),
      id: existing?.id ?? null,
      calendar_id: f.cal.value,
      title: f.title.value.trim(),
      start: s.toISOString(),
      end: e.toISOString(),
      all_day: all,
      timezone: all ? null : zone,
      rrule: repeat.value(),
      location: f.location.value.trim(),
      description: f.desc.value,
      attendees,
      alerts: alerts.value(),
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

  const line = (name: string, label: string, ...body: (Node | false)[]) => h('div.ev-line', h('span.ev-icon', { title: label, 'aria-hidden': 'true' }, icon(name, 18)), h('div.ev-body', ...body));
  const m = modal(existing ? 'Edit event' : 'New event', h('div.event-form',
    f.title,
    line('clock', 'When',
      h('div.ev-when',
        h('span.muted', 'Starts'), f.startDate, f.startTime,
        h('span.muted', 'Ends'), f.endDate, f.endTime),
      h('div.ev-opts', h('label.row.inline', f.allDay, 'All day'), zoneRow),
      hint),
    line('refresh', 'Repeat', repeat.el),
    line('bell', 'Alerts', alerts.el),
    line('location', 'Location', locationRow),
    line('user', 'Guests', f.attendees),
    line('calendar', 'Calendar', f.cal, hasGuests && h('div.muted.small', 'Events with guests stay in their calendar, so guests aren’t sent a cancellation.')),
    line('notes', 'Description', f.desc)), {
    footer: h('div.row.end',
      h('button', { title: 'Close without saving (Esc)', on: { click: () => m.close() } }, 'Cancel'),
      h('button.primary', { title: existing ? 'Save changes' : 'Create the event', on: { click: () => void save() } }, existing ? 'Save' : 'Create')),
  });
  f.title.focus();
}

/** The calendars to choose from, grouped by account when there's more than one. */
function calendarOptions(cals: Calendar[], selected?: string) {
  const option = (c: Calendar) => h('option', { value: c.id, selected: c.id === selected }, c.name);
  const accounts = [...new Set(cals.map((c) => c.account_id))];
  if (accounts.length < 2) return cals.map(option);
  const email = (id: string) => store.state.accounts.find((a) => a.id === id)?.email ?? 'Other';
  return accounts.map((id) => h('optgroup', { label: email(id) }, cals.filter((c) => c.account_id === id).map(option)));
}

const PLACE_ICONS: Record<PlaceSource, string> = { history: 'clock', open_street_map: 'location', apple_maps: 'location' };
/** Lookups wait for a pause in typing; shorter text only searches history. */
const LOOKUP_PAUSE = 250;
const LOOKUP_FROM = 3;

/** Places you've used before, then (if turned on in Privacy) a place
 *  service's, crediting whichever answered. While lookups are off, a button
 *  beside the field offers to turn them on. */
function locationPicker(input: HTMLInputElement) {
  const offer = h('button.icon-btn', { type: 'button', title: 'Look up places as you type…', hidden: !!store.state.settings?.look_up_places, on: { click: () => void turnOn() } }, icon('search', 16));
  const turnOn = async () => {
    const service = (await api.info().catch(() => null))?.places_service ?? 'a place service';
    const yes = await confirmDialog('Look up places?', h('div',
      h('p', 'Satchel can suggest real places, with their addresses, as you type an event’s location.'),
      h('p.muted', `What you type in Location is sent to ${service}. Places you’ve used before are suggested either way, without leaving this device. You can change this in Settings → Privacy.`)), 'Turn on');
    const settings = store.state.settings;
    if (!yes || !settings) return input.focus();
    try {
      store.set({ settings: await api.updateSettings({ ...settings, look_up_places: true }) });
      offer.hidden = true;
      toast(`Places now come from ${service} too`);
    } catch (e) {
      errorToast(e);
    }
    // Asks again, now with lookups.
    input.focus();
  };

  let sources: PlaceSource[] = [];
  let typed = 0;
  combobox(input, async (q) => {
    const n = ++typed;
    if (q.trim().length >= LOOKUP_FROM) await new Promise((r) => setTimeout(r, LOOKUP_PAUSE));
    if (n !== typed) return [];
    const found = await api.suggestLocations(q).catch(() => []);
    sources = found.map((p) => p.source);
    return found.map((p) => ({ label: p.label, detail: p.detail ?? undefined, icon: PLACE_ICONS[p.source], value: p.text }));
  }, (s) => { input.value = s.value; }, {
    freeText: true,
    twoLine: true,
    footer: () => (sources.includes('open_street_map') ? '© OpenStreetMap contributors' : sources.includes('apple_maps') ? 'Places from Apple Maps' : null),
  });
  return h('div.ev-location', input, offer);
}

/** Search every time zone by city, region, name ("pacific") or offset ("+5:30"). */
function zonePicker(input: HTMLInputElement, current: () => string, label: () => string, pick: (tz: string) => void) {
  let zones: { tz: string; off: number; text: string; s: Suggestion }[] | null = null;
  // Until something's typed, list the current and local zones first.
  let typed = false;
  const load = () => {
    const now = new Date();
    zones ??= allZones()
      .map((tz) => {
        const off = offsetLabel(tz, now);
        const region = tz.includes('/') ? tz.split('/')[0] : '';
        return { tz, off: offsetMinutes(tz, now), text: `${zoneCity(tz)} ${tz.replace(/_/g, ' ')} ${zoneLongName(tz, now)} ${off}`, s: { label: zoneCity(tz), detail: [region, off].filter(Boolean).join(' · '), value: tz } };
      })
      .sort((a, b) => a.off - b.off || a.tz.localeCompare(b.tz));
    return zones;
  };
  // Before combobox's own listeners, which ask for suggestions.
  listen(input, 'focus', () => { typed = false; input.select(); });
  listen(input, 'input', () => { typed = true; });
  combobox(input, (q) => {
    const all = load();
    if (!typed || !q.trim()) {
      const top = [...new Set([current(), localZone])];
      return [...top.map((tz) => all.find((z) => z.tz === tz)!).filter(Boolean), ...all.filter((z) => !top.includes(z.tz))].map((z) => z.s);
    }
    // Substring hits ("berl" → Berlin) when there are any, else fuzzy ones.
    const lower = q.trim().toLowerCase();
    const plain = all.filter((z) => z.text.toLowerCase().includes(lower));
    return (plain.length ? plain : all)
      .map((z) => ({ z, m: fuzzy(q, z.text) }))
      .filter((x) => x.m)
      .sort((a, b) => b.m!.score - a.m!.score)
      .slice(0, 50)
      .map((x) => x.z.s);
  }, (s) => {
    pick(s.value);
    typed = false;
    input.select();
  });
  // Leaving without picking puts the current zone back.
  listen(input, 'blur', () => { input.value = label(); });
}
