// Calendar: day / week / month / agenda views, event details, RSVP. The
// editor and right-click menus live in calendar-editor.ts and calendar-menus.ts.

import { api } from '../api';
import { h, icon, replace } from '../lib/dom';
import { timeRange } from '../lib/format';
import { bind, confirmDialog, contextMenu, errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { CalEvent, Calendar, Occurrence, PartStat } from '../types';
import { editEvent } from './calendar-editor';
import { overflowMenu } from '../lib/overflow';
import { swipePager } from '../lib/swipe';
import { calendarMenu, eventMenu, slotMenu } from './calendar-menus';
import type { CalendarActions } from './calendar-menus';

type Mode = 'day' | 'week' | 'month' | 'agenda';
const HOUR_PX = 48;
const SIDE_KEY = 'satchel.calendarSideCollapsed';

function readSideCollapsed() {
  try {
    return localStorage.getItem(SIDE_KEY) === '1';
  } catch {
    return false;
  }
}

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const startOfWeek = (d: Date) => addDays(startOfDay(d), -((d.getDay() + 6) % 7));
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();

export class CalendarView {
  root = h('div.calendar-view');
  private mode: Mode = 'week';
  private anchor = startOfDay(new Date());
  private calendars: Calendar[] = [];
  private occurrences: Occurrence[] = [];
  private sideCollapsed = readSideCollapsed();
  /** The previous, current and next periods side by side; slides when swiping. */
  private body?: HTMLElement;
  private act: CalendarActions = {
    details: (o) => this.details(o),
    editEvent: (o) => void api.getEvent(o.event.id!).then((full) => this.edit(full), errorToast),
    respond: (o, r) => void this.respond(o.event, r),
    deleteEvent: (o) => void this.deleteEvent(o.event),
    newEvent: (at, allDay) => this.edit(null, at, allDay),
    openDay: (d) => { this.anchor = startOfDay(d); this.setMode('day'); },
    setVisible: (visible) => void this.setVisible(visible),
  };

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
    bind('[', 'Collapse or expand calendar list', () => this.toggleSide(), s);
    swipePager(this.root, () => this.body, (dir) => this.step(dir));
  }

  private toggleSide() {
    this.sideCollapsed = !this.sideCollapsed;
    try {
      localStorage.setItem(SIDE_KEY, this.sideCollapsed ? '1' : '0');
    } catch {
      /* remembering is a convenience */
    }
    this.render();
  }

  show() {
    void this.load();
  }

  private setMode(m: Mode) {
    this.mode = m;
    void this.load();
  }

  /** The anchor `dir` periods away from the current one. */
  private shifted(dir: number): Date {
    const a = this.anchor;
    return this.mode === 'day' ? addDays(a, dir) : this.mode === 'week' ? addDays(a, 7 * dir) : this.mode === 'month' ? new Date(a.getFullYear(), a.getMonth() + dir, 1) : addDays(a, 14 * dir);
  }

  private step(dir: number) {
    this.anchor = this.shifted(dir);
    return this.load();
  }

  private range(a = this.anchor): [Date, Date] {
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
    // The neighbouring periods too, so they show while swiping.
    const [from] = this.range(this.shifted(-1));
    const [, to] = this.range(this.shifted(1));
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
    const view = (a: Date) => (this.mode === 'month' ? this.month(a) : this.mode === 'agenda' ? this.agenda(a) : this.timeGrid(this.mode === 'day' ? 1 : 7, a));
    const pane = (dir: number) => {
      const el = h(`div.cal-pane${dir < 0 ? '.prev' : dir > 0 ? '.next' : '.cur'}`, view(this.shifted(dir)));
      // Neighbours are only there to be seen mid-swipe.
      if (dir) el.inert = true;
      return el;
    };
    const body = h('div.cal-track', pane(-1), pane(0), pane(1));
    // Keep the time grid's scroll position when moving between periods.
    const scrolled = this.root.querySelector('.cal-pane.cur .time-scroll')?.scrollTop;
    this.body = body;
    this.root.classList.toggle('side-collapsed', this.sideCollapsed);
    const name = (m: Mode) => m[0].toUpperCase() + m.slice(1);
    const tabs = modes.map((m) => ({
      el: h(`button${this.mode === m ? '.active' : ''}`, { role: 'tab', title: `${name(m)} view (${m[0]})`, on: { click: () => this.setMode(m) } }, name(m)),
      item: () => ({ label: `${name(m)} view`, hint: m[0], icon: this.mode === m ? 'check' : undefined, run: () => this.setMode(m) }),
      pinned: this.mode === m,
    }));
    const title = h('h1', this.title());
    const head = h('header.cal-head',
      h('button.icon-btn.cal-side-toggle', {
        title: `${this.sideCollapsed ? 'Show' : 'Hide'} calendar list ([)`,
        'aria-label': this.sideCollapsed ? 'Show calendar list' : 'Hide calendar list',
        'aria-expanded': String(!this.sideCollapsed),
        on: { click: () => this.toggleSide() },
      }, icon('panel')),
      h('button.icon-btn.cal-new', { title: 'New event (n)', 'aria-label': 'New event', on: { click: () => this.edit(null) } }, icon('plus')),
      h('button', { title: 'Go to today (t)', on: { click: () => { this.anchor = startOfDay(new Date()); void this.load(); } } }, 'Today'),
      h('button.icon-btn', { title: 'Previous (k)', on: { click: () => this.step(-1) } }, icon('chevronLeft')),
      h('button.icon-btn', { title: 'Next (j)', on: { click: () => this.step(1) } }, icon('chevronRight')),
      title,
      h('span.spacer'),
      h('div.segmented', { role: 'tablist' }, tabs.map((t) => t.el)));
    head.append(overflowMenu(head, tabs, [title]));
    replace(
      this.root,
      !this.sideCollapsed && h('aside.cal-side',
        h('button.primary', { title: 'Create an event (n)', on: { click: () => this.edit(null) } }, icon('plus', 16), 'New event'),
        this.miniMonth(),
        h('div.cal-list',
          h('div.mb-section', 'Calendars'),
          this.calendars.length === 0 && h('p.muted.small', 'No calendars yet. Calendars sync from accounts that support them.'),
          this.calendars.map((c) =>
            this.withMenu(h('label.cal-toggle',
              h('input', { type: 'checkbox', checked: c.visible, on: { change: async (e: Event) => { await api.setCalendarVisible(c.id, (e.target as HTMLInputElement).checked); void this.load(); } } }),
              h('span.swatch', { style: { background: c.color } }),
              h('span', c.name),
              c.read_only && h('span.muted.small', ' (read-only)')), () => calendarMenu(this.act, c, this.calendars))))),
      h('section.cal-main', head, body),
    );
    const grids = [...body.querySelectorAll<HTMLElement>('.time-scroll')];
    for (const g of grids) g.scrollTop = scrolled || HOUR_PX * 7.5;
    const cur = body.querySelector<HTMLElement>('.cal-pane.cur .time-scroll');
    cur?.addEventListener('scroll', () => grids.forEach((g) => g !== cur && (g.scrollTop = cur.scrollTop)), { passive: true });
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
    return this.withMenu(h(`button.event-chip${declined ? '.declined' : ''}${needs ? '.needs' : ''}${o.event.status === 'tentative' ? '.tentative' : ''}`,
      { style: { '--ev': o.color } as unknown as Partial<CSSStyleDeclaration>, title: `${o.event.title}\n${timeRange(o.start, o.end, o.event.all_day)}`, on: { click: (e: MouseEvent) => { e.stopPropagation(); this.details(o); } } },
      !compact && !o.event.all_day && h('span.ev-time', new Date(o.start).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })),
      h('span.ev-title', o.event.title)), o);
  }

  /** Attach a right-click menu built by `build`, or the event menu for `o`. */
  private withMenu<T extends HTMLElement>(el: T, build: Occurrence | ((e: MouseEvent) => ReturnType<typeof eventMenu>)): T {
    contextMenu(el, typeof build === 'function' ? build : () => eventMenu(this.act, build, this.calendars.find((c) => c.id === build.event.calendar_id)));
    return el;
  }

  private timeGrid(days: number, a: Date): HTMLElement {
    const [from] = this.range(a);
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
      // The half hour under the pointer.
      const slotAt = (e: MouseEvent) => {
        const y = e.clientY - column.getBoundingClientRect().top;
        return new Date(d.getFullYear(), d.getMonth(), d.getDate(), Math.floor(y / HOUR_PX), y % HOUR_PX >= HOUR_PX / 2 ? 30 : 0);
      };
      const column = h(`div.day-col${sameDay(d, today) ? '.today' : ''}`, {
        on: {
          dblclick: (e: MouseEvent) => this.edit(null, slotAt(e)),
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
      return this.withMenu(column, (e) => slotMenu(this.act, slotAt(e), true, days === 1));
    };
    return h(`div.time-grid.days-${days}`,
      h('div.grid-head',
        h('div.gutter'),
        dates.map((d) => h(`div.day-head${sameDay(d, today) ? '.today' : ''}`, h('span.dow', d.toLocaleDateString([], { weekday: 'short' })), h('span.dom', String(d.getDate()))))),
      h('div.allday-row', h('div.gutter.muted.small', 'all-day'), dates.map((d) => this.withMenu(h('div.allday-cell', allDay(d).map((o) => this.chip(o, true))), () => slotMenu(this.act, d, false, days === 1)))),
      h('div.time-scroll',
        h('div.time-body',
          h('div.gutter.hours', Array.from({ length: 24 }, (_, hr) => h('div.hour-label', { style: { top: `${hr * HOUR_PX}px` } }, hr === 0 ? '' : new Date(2000, 0, 1, hr).toLocaleTimeString([], { hour: 'numeric' })))),
          dates.map(col))));
  }

  private month(a: Date): HTMLElement {
    const [from] = this.range(a);
    const today = new Date();
    return h('div.month-grid',
      ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((d) => h('div.month-dow', d)),
      Array.from({ length: 42 }, (_, i) => {
        const d = addDays(from, i);
        const evs = this.eventsOn(d).sort((a, b) => Number(b.event.all_day) - Number(a.event.all_day) || a.start.localeCompare(b.start));
        return this.withMenu(h(`div.month-cell${d.getMonth() !== a.getMonth() ? '.other' : ''}${sameDay(d, today) ? '.today' : ''}`,
          { on: { dblclick: () => this.edit(null, new Date(d.getFullYear(), d.getMonth(), d.getDate(), 9)) } },
          h('button.month-date', { title: 'Open this day', on: { click: () => { this.anchor = d; this.setMode('day'); } } }, String(d.getDate())),
          evs.slice(0, 4).map((o) => this.chip(o)),
          evs.length > 4 && h('span.more.muted.small', `+${evs.length - 4} more`)), () => slotMenu(this.act, d, false, false));
      }));
  }

  private agenda(a: Date): HTMLElement {
    const [from, to] = this.range(a);
    const days: HTMLElement[] = [];
    for (let d = from; d < to; d = addDays(d, 1)) {
      const evs = this.eventsOn(d);
      if (!evs.length) continue;
      days.push(h('section.agenda-day',
        h('h3', d.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })),
        evs.map((o) => this.withMenu(h('button.agenda-item', { on: { click: () => this.details(o) } },
          h('span.swatch', { style: { background: o.color } }),
          h('span.agenda-time', timeRange(o.start, o.end, o.event.all_day)),
          h('span.agenda-title', o.event.title),
          o.event.location && h('span.muted.small', o.event.location),
          o.event.my_status === 'needs_action' && h('span.status-chip.pending', 'needs response')), o))));
    }
    return h('div.agenda', days.length ? days : h('div.empty-state', icon('calendar', 36), h('p', 'No events in the next 30 days.')));
  }

  private details(o: Occurrence) {
    const e = o.event;
    const cal = this.calendars.find((c) => c.id === e.calendar_id);
    const respond = async (r: PartStat) => {
      if (await this.respond(e, r)) m.close();
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
                if (await this.deleteEvent(e)) m.close();
              },
            },
          }, 'Delete'),
          h('span.spacer'),
          !cal?.read_only && h('button.primary', { on: { click: async () => { m.close(); const full = await api.getEvent(e.id!); this.edit(full); } } }, 'Edit')),
      });
  }

  private async respond(e: CalEvent, r: PartStat): Promise<boolean> {
    try {
      await api.respondToInvite(e.id!, r);
      toast('Response saved', { quiet: true });
      void this.load();
      return true;
    } catch (err) {
      errorToast(err);
      return false;
    }
  }

  /** Delete after asking; true when it was deleted. */
  private async deleteEvent(e: CalEvent): Promise<boolean> {
    if (!(await confirmDialog('Delete event?', e.rrule ? 'This deletes every occurrence of this repeating event.' : `Delete “${e.title}”?`, 'Delete', true))) return false;
    try {
      await api.deleteEvent(e.id!);
      void this.load();
      return true;
    } catch (err) {
      errorToast(err);
      return false;
    }
  }

  private async setVisible(visible: (c: Calendar) => boolean) {
    try {
      await Promise.all(this.calendars.filter((c) => visible(c) !== c.visible).map((c) => api.setCalendarVisible(c.id, visible(c))));
    } catch (err) {
      errorToast(err);
    }
    void this.load();
  }

  private edit(existing: CalEvent | null, at?: Date, allDay = false) {
    editEvent(this.calendars, existing, () => void this.load(), at, allDay);
  }
}
