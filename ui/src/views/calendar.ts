// Calendar: day / week / month / agenda views, event editor, RSVP.

import { api } from '../api';
import { h, icon, replace } from '../lib/dom';
import { parseAddresses, timeRange } from '../lib/format';
import { bind, confirmDialog, errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { CalEvent, Calendar, Occurrence, PartStat } from '../types';

type Mode = 'day' | 'week' | 'month' | 'agenda';
const HOUR_PX = 48;
const localTz = Intl.DateTimeFormat().resolvedOptions().timeZone;

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const startOfWeek = (d: Date) => addDays(startOfDay(d), -((d.getDay() + 6) % 7));
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();

function toLocalInput(d: Date) {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

export class CalendarView {
  root = h('div.calendar-view');
  private mode: Mode = 'week';
  private anchor = startOfDay(new Date());
  private calendars: Calendar[] = [];
  private occurrences: Occurrence[] = [];

  constructor() {
    store.onEvent((e) => {
      if ((e.event === 'calendar_changed' || e.event === 'data_changed') && store.state.view === 'calendar') void this.load();
    });
    const s = 'calendar';
    bind('t', 'Today', () => { this.anchor = startOfDay(new Date()); void this.load(); }, s);
    bind('j', 'Next period', () => this.step(1), s);
    bind('k', 'Previous period', () => this.step(-1), s);
    bind('ArrowRight', 'Next period', () => this.step(1), s);
    bind('ArrowLeft', 'Previous period', () => this.step(-1), s);
    bind('d', 'Day view', () => this.setMode('day'), s);
    bind('w', 'Week view', () => this.setMode('week'), s);
    bind('m', 'Month view', () => this.setMode('month'), s);
    bind('a', 'Agenda view', () => this.setMode('agenda'), s);
    bind('n', 'New event', () => this.edit(null), s);
  }

  show() {
    void this.load();
  }

  private setMode(m: Mode) {
    this.mode = m;
    void this.load();
  }

  private step(dir: number) {
    const a = this.anchor;
    this.anchor =
      this.mode === 'day' ? addDays(a, dir) : this.mode === 'week' ? addDays(a, 7 * dir) : this.mode === 'month' ? new Date(a.getFullYear(), a.getMonth() + dir, 1) : addDays(a, 14 * dir);
    void this.load();
  }

  private range(): [Date, Date] {
    const a = this.anchor;
    switch (this.mode) {
      case 'day':
        return [a, addDays(a, 1)];
      case 'week':
        return [startOfWeek(a), addDays(startOfWeek(a), 7)];
      case 'month': {
        const first = new Date(a.getFullYear(), a.getMonth(), 1);
        const start = startOfWeek(first);
        return [start, addDays(start, 42)];
      }
      case 'agenda':
        return [a, addDays(a, 30)];
    }
  }

  private async load() {
    const [from, to] = this.range();
    try {
      const [cals, occ] = await Promise.all([api.listCalendars(), api.listEvents(from.toISOString(), to.toISOString())]);
      this.calendars = cals;
      this.occurrences = occ;
    } catch (e) {
      errorToast(e);
    }
    this.render();
  }

  private title(): string {
    const [from, to] = this.range();
    if (this.mode === 'day') return from.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' });
    if (this.mode === 'month') return this.anchor.toLocaleDateString([], { month: 'long', year: 'numeric' });
    const end = addDays(to, -1);
    return `${from.toLocaleDateString([], { month: 'short', day: 'numeric' })} – ${end.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })}`;
  }

  private render() {
    const modes: Mode[] = ['day', 'week', 'month', 'agenda'];
    const body = this.mode === 'month' ? this.month() : this.mode === 'agenda' ? this.agenda() : this.timeGrid(this.mode === 'day' ? 1 : 7);
    replace(
      this.root,
      h('aside.cal-side',
        h('button.primary', { title: 'Create an event', on: { click: () => this.edit(null) } }, icon('plus', 16), 'New event'),
        this.miniMonth(),
        h('div.cal-list',
          h('div.mb-section', 'Calendars'),
          this.calendars.length === 0 && h('p.muted.small', 'No calendars yet. Calendars sync from accounts that support them.'),
          this.calendars.map((c) =>
            h('label.cal-toggle',
              h('input', { type: 'checkbox', checked: c.visible, on: { change: async (e: Event) => { await api.setCalendarVisible(c.id, (e.target as HTMLInputElement).checked); void this.load(); } } }),
              h('span.swatch', { style: { background: c.color } }),
              h('span', c.name),
              c.read_only && h('span.muted.small', ' (read-only)'))))),
      h('section.cal-main',
        h('header.cal-head',
          h('button', { title: 'Go to today (t)', on: { click: () => { this.anchor = startOfDay(new Date()); void this.load(); } } }, 'Today'),
          h('button.icon-btn', { title: 'Previous (k)', on: { click: () => this.step(-1) } }, icon('chevronLeft')),
          h('button.icon-btn', { title: 'Next (j)', on: { click: () => this.step(1) } }, icon('chevronRight')),
          h('h1', this.title()),
          h('span.spacer'),
          h('div.segmented', { role: 'tablist' }, modes.map((m) => h(`button${this.mode === m ? '.active' : ''}`, { role: 'tab', title: `${m[0].toUpperCase() + m.slice(1)} view (${m[0]})`, on: { click: () => this.setMode(m) } }, m[0].toUpperCase() + m.slice(1))))),
        body),
    );
    const grid = this.root.querySelector('.time-scroll');
    if (grid && !grid.scrollTop) grid.scrollTop = HOUR_PX * 7.5;
  }

  private miniMonth(): HTMLElement {
    const a = this.anchor;
    const first = new Date(a.getFullYear(), a.getMonth(), 1);
    const start = startOfWeek(first);
    const today = new Date();
    const cells = [];
    for (let i = 0; i < 42; i++) {
      const d = addDays(start, i);
      cells.push(h(`button.mini-day${d.getMonth() !== a.getMonth() ? '.other' : ''}${sameDay(d, today) ? '.today' : ''}${sameDay(d, a) ? '.sel' : ''}`, { title: d.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' }), on: { click: () => { this.anchor = d; void this.load(); } } }, String(d.getDate())));
    }
    return h('div.mini-month',
      h('div.mini-head', a.toLocaleDateString([], { month: 'long', year: 'numeric' })),
      h('div.mini-grid', ['M', 'T', 'W', 'T', 'F', 'S', 'S'].map((d) => h('span.mini-dow', d)), cells));
  }

  private eventsOn(day: Date) {
    const next = addDays(day, 1);
    return this.occurrences.filter((o) => new Date(o.start) < next && new Date(o.end) > day);
  }

  private chip(o: Occurrence, compact = false): HTMLElement {
    const declined = o.event.my_status === 'declined';
    const needs = o.event.my_status === 'needs_action';
    return h(`button.event-chip${declined ? '.declined' : ''}${needs ? '.needs' : ''}${o.event.status === 'tentative' ? '.tentative' : ''}`,
      { style: { '--ev': o.color } as unknown as Partial<CSSStyleDeclaration>, title: `${o.event.title}\n${timeRange(o.start, o.end, o.event.all_day)}`, on: { click: (e: MouseEvent) => { e.stopPropagation(); this.details(o); } } },
      !compact && !o.event.all_day && h('span.ev-time', new Date(o.start).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })),
      h('span.ev-title', o.event.title));
  }

  private timeGrid(days: number): HTMLElement {
    const [from] = this.range();
    const dates = Array.from({ length: days }, (_, i) => addDays(from, i));
    const today = new Date();
    const allDay = (d: Date) => this.eventsOn(d).filter((o) => o.event.all_day || new Date(o.end).getTime() - new Date(o.start).getTime() >= 86_400_000);
    const timed = (d: Date) => this.eventsOn(d).filter((o) => !o.event.all_day && new Date(o.end).getTime() - new Date(o.start).getTime() < 86_400_000);
    const col = (d: Date) => {
      const items = timed(d).sort((a, b) => a.start.localeCompare(b.start) || b.end.localeCompare(a.end));
      // Overlap layout: events that overlap form a cluster; each cluster splits
      // its width into as many lanes as it needs.
      const placed: { o: Occurrence; lane: number; lanes: number }[] = [];
      let cluster: { o: Occurrence; lane: number; lanes: number }[] = [];
      let laneEnds: number[] = [];
      let clusterEnd = 0;
      const flush = () => {
        for (const c of cluster) c.lanes = laneEnds.length;
        placed.push(...cluster);
        cluster = [];
        laneEnds = [];
      };
      for (const o of items) {
        const s = new Date(o.start).getTime();
        const e = Math.max(new Date(o.end).getTime(), s + 20 * 60_000);
        if (cluster.length && s >= clusterEnd) flush();
        let lane = laneEnds.findIndex((end) => end <= s);
        if (lane < 0) {
          lane = laneEnds.length;
          laneEnds.push(0);
        }
        laneEnds[lane] = e;
        clusterEnd = Math.max(clusterEnd, e);
        cluster.push({ o, lane, lanes: 1 });
      }
      flush();
      const dayStart = startOfDay(d).getTime();
      const column = h(`div.day-col${sameDay(d, today) ? '.today' : ''}`, {
        on: {
          dblclick: (e: MouseEvent) => {
            const y = e.offsetY;
            const hour = Math.floor(y / HOUR_PX);
            const start = new Date(d.getFullYear(), d.getMonth(), d.getDate(), hour, y % HOUR_PX >= HOUR_PX / 2 ? 30 : 0);
            this.edit(null, start);
          },
        },
      },
        Array.from({ length: 24 }, (_, hr) => h('div.hour-line', { style: { top: `${hr * HOUR_PX}px` } })),
        placed.map(({ o, lane, lanes }) => {
          const width = 100 / lanes;
          const s = Math.max(new Date(o.start).getTime(), dayStart);
          const e = Math.min(new Date(o.end).getTime(), dayStart + 86_400_000);
          const top = ((s - dayStart) / 3_600_000) * HOUR_PX;
          const height = Math.max(((e - s) / 3_600_000) * HOUR_PX - 2, 18);
          const el = this.chip(o);
          el.classList.add('timed');
          Object.assign(el.style, { top: `${top}px`, height: `${height}px`, left: `${lane * width}%`, width: `calc(${width}% - 3px)` });
          return el;
        }),
        sameDay(d, today) && h('div.now-line', { style: { top: `${((today.getHours() * 60 + today.getMinutes()) / 60) * HOUR_PX}px` } }));
      return column;
    };
    return h(`div.time-grid.days-${days}`,
      h('div.grid-head',
        h('div.gutter'),
        dates.map((d) => h(`div.day-head${sameDay(d, today) ? '.today' : ''}`, h('span.dow', d.toLocaleDateString([], { weekday: 'short' })), h('span.dom', String(d.getDate()))))),
      h('div.allday-row', h('div.gutter.muted.small', 'all-day'), dates.map((d) => h('div.allday-cell', allDay(d).map((o) => this.chip(o, true))))),
      h('div.time-scroll',
        h('div.time-body',
          h('div.gutter.hours', Array.from({ length: 24 }, (_, hr) => h('div.hour-label', { style: { top: `${hr * HOUR_PX}px` } }, hr === 0 ? '' : new Date(2000, 0, 1, hr).toLocaleTimeString([], { hour: 'numeric' })))),
          dates.map(col))));
  }

  private month(): HTMLElement {
    const [from] = this.range();
    const today = new Date();
    return h('div.month-grid',
      ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((d) => h('div.month-dow', d)),
      Array.from({ length: 42 }, (_, i) => {
        const d = addDays(from, i);
        const evs = this.eventsOn(d).sort((a, b) => Number(b.event.all_day) - Number(a.event.all_day) || a.start.localeCompare(b.start));
        return h(`div.month-cell${d.getMonth() !== this.anchor.getMonth() ? '.other' : ''}${sameDay(d, today) ? '.today' : ''}`,
          { on: { dblclick: () => this.edit(null, new Date(d.getFullYear(), d.getMonth(), d.getDate(), 9)) } },
          h('button.month-date', { title: 'Open this day', on: { click: () => { this.anchor = d; this.setMode('day'); } } }, String(d.getDate())),
          evs.slice(0, 4).map((o) => this.chip(o)),
          evs.length > 4 && h('span.more.muted.small', `+${evs.length - 4} more`));
      }));
  }

  private agenda(): HTMLElement {
    const [from, to] = this.range();
    const days: HTMLElement[] = [];
    for (let d = from; d < to; d = addDays(d, 1)) {
      const evs = this.eventsOn(d);
      if (!evs.length) continue;
      days.push(h('section.agenda-day',
        h('h3', d.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })),
        evs.map((o) => h('button.agenda-item', { on: { click: () => this.details(o) } },
          h('span.swatch', { style: { background: o.color } }),
          h('span.agenda-time', timeRange(o.start, o.end, o.event.all_day)),
          h('span.agenda-title', o.event.title),
          o.event.location && h('span.muted.small', o.event.location),
          o.event.my_status === 'needs_action' && h('span.status-chip.pending', 'needs response')))));
    }
    return h('div.agenda', days.length ? days : h('div.empty-state', icon('calendar', 36), h('p', 'No events in the next 30 days.')));
  }

  private details(o: Occurrence) {
    const e = o.event;
    const cal = this.calendars.find((c) => c.id === e.calendar_id);
    const respond = async (r: PartStat) => {
      try {
        await api.respondToInvite(e.id!, r);
        m.close();
        toast('Response saved');
        void this.load();
      } catch (err) {
        errorToast(err);
      }
    };
    const m = modal(e.title, h('div.event-details',
      h('div', icon('clock', 16), ` ${new Date(o.start).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })} · ${timeRange(o.start, o.end, e.all_day)}`),
      e.rrule && h('div.muted.small', `Repeats (${e.rrule})`),
      e.location && h('div', icon('location', 16), ` ${e.location}`),
      cal && h('div.muted.small', h('span.swatch', { style: { background: cal.color } }), ` ${cal.name}`),
      e.organizer && h('div.small', `Organizer: ${e.organizer.name ?? e.organizer.email}`),
      e.attendees.length > 0 && h('ul.attendees', e.attendees.map((a) => h('li', h(`span.rsvp.${a.status}`), a.name ?? a.email, h('span.muted.small', ` ${a.status.replace('_', ' ')}`)))),
      e.description && h('p.event-desc', e.description),
      e.my_status && h('div.rsvp-bar',
        h('span', 'Going?'),
        (['accepted', 'tentative', 'declined'] as PartStat[]).map((r) => h(`button${e.my_status === r ? '.active' : ''}`, { on: { click: () => void respond(r) } }, r === 'accepted' ? 'Yes' : r === 'declined' ? 'No' : 'Maybe')))),
      {
        footer: h('div.row.end',
          !cal?.read_only && h('button.danger', {
            on: {
              click: async () => {
                if (!(await confirmDialog('Delete event?', e.rrule ? 'This deletes every occurrence of this repeating event.' : `Delete “${e.title}”?`, 'Delete', true))) return;
                try {
                  await api.deleteEvent(e.id!);
                  m.close();
                  void this.load();
                } catch (err) {
                  errorToast(err);
                }
              },
            },
          }, 'Delete'),
          h('span.spacer'),
          !cal?.read_only && h('button.primary', { on: { click: async () => { m.close(); const full = await api.getEvent(e.id!); this.edit(full); } } }, 'Edit')),
      });
  }

  private edit(existing: CalEvent | null, at?: Date) {
    const writable = this.calendars.filter((c) => !c.read_only);
    if (!writable.length) {
      toast('No writable calendars. Add an account with calendar access first.', { kind: 'error' });
      return;
    }
    const start = existing ? new Date(existing.start) : at ?? new Date(Math.ceil(Date.now() / 1_800_000) * 1_800_000);
    const end = existing ? new Date(existing.end) : new Date(start.getTime() + 3_600_000);
    const f = {
      title: h('input', { type: 'text', value: existing?.title ?? '', placeholder: 'Add title', 'aria-label': 'Title' }),
      cal: h('select', { 'aria-label': 'Calendar', disabled: !!existing }, writable.map((c) => h('option', { value: c.id, selected: c.id === existing?.calendar_id }, c.name))),
      allDay: h('input', { type: 'checkbox', checked: existing?.all_day ?? false }),
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
        void this.load();
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
}
