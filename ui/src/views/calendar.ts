// Calendar: day / week / month / agenda views, event details, RSVP. The
// editor and right-click menus live in calendar-editor.ts and calendar-menus.ts.

import { api } from '../api';
import { h, icon, patch } from '../lib/dom';
import { timeRange } from '../lib/format';
import { bind, confirmDialog, contextMenu, errorToast, modal, toast } from '../lib/ui';
import { store } from '../state';
import type { CalEvent, Calendar, Occurrence, PartStat } from '../types';
import { editEvent } from './calendar-editor';
import { overflowMenu } from '../lib/overflow';
import { lockWheelAxis, swipePager } from '../lib/swipe';
import { calendarMenu, eventMenu, slotMenu } from './calendar-menus';
import type { CalendarActions } from './calendar-menus';
import { eventRepeatText } from '../lib/repeat-text';
import { alertLabel } from '../lib/alerts';

type Mode = 'day' | 'week' | 'month' | 'agenda';
const MODES: Mode[] = ['day', 'week', 'month', 'agenda'];
const modeName = (m: Mode) => m[0].toUpperCase() + m.slice(1);
const HOUR_PX = 48;
// The week view is a strip of days that scrolls sideways, a week to the
// screen. It's laid out STRIP_ORIGIN_DAYS either side of
// where it opened, so scrolling never has to move it (moving it mid-scroll
// flickers), but only STRIP_DRAW_DAYS either side of the days in view are
// drawn and loaded, again once the view drifts STRIP_DRIFT_DAYS from them.
const STRIP_ORIGIN_DAYS = 365;
const STRIP_GRID_DAYS = 2 * STRIP_ORIGIN_DAYS + 7;
const STRIP_DRAW_DAYS = 28;
const STRIP_DRIFT_DAYS = 14;
/** Whether the sidebar's calendar list is open; it starts shut. */
const LIST_OPEN_KEY = 'satchel.calendarListOpen';

function readListOpen() {
  try {
    return localStorage.getItem(LIST_OPEN_KEY) === '1';
  } catch {
    return false;
  }
}
const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const startOfWeek = (d: Date) => addDays(startOfDay(d), -((d.getDay() + 6) % 7));
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
/** Whether `m` is drawn as the sideways-scrolling strip of days; the others page. */
const isStrip = (m: Mode) => m === 'week';
/** The six weeks a month grid shows for the month holding `a`. */
function monthGrid(a: Date): [Date, Date] {
  const start = startOfWeek(new Date(a.getFullYear(), a.getMonth(), 1));
  return [start, addDays(start, 42)];
}
const daysBetween = (a: Date, b: Date) => Math.round((b.getTime() - a.getTime()) / 86_400_000);

export class CalendarView {
  root = h('div.calendar-view');
  private mode: Mode = 'week';
  private anchor = startOfWeek(new Date());
  private calendars: Calendar[] = [];
  private occurrences: Occurrence[] = [];
  /** Week view: how far past `anchor` (the first day in view) it's scrolled, in days (0–1). */
  private stripFrac = 0;
  /** Week view: the day in the strip's first column. */
  private stripOrigin = this.anchor;
  /** Week view: the first day drawn. */
  private stripFrom = this.anchor;
  /** Week view: the day width the strip was last placed at. */
  private stripDayWidth = 0;
  /** Week view: drawing around a new position is under way. */
  private redrawing = false;
  /** How far down the hours are scrolled; kept here because hiding the view
   * resets the grids' scroll positions. */
  private hourScroll = HOUR_PX * 7.5;
  /** Put the scroll positions back on the next draw (after navigating, or the
   * view being hidden); until then, scroll events don't say where the user is. */
  private placeNext = true;
  private loads = 0;
  /** The month and the calendar list, shown in the app sidebar. */
  side = h('aside.cal-side');
  private listOpen = readListOpen();
  /** The day view's agenda for the day, beside the day. */
  private dayPanel = h('aside.day-panel');
  /** The previous, current and next periods side by side; slides when swiping. */
  private track = h('div.cal-track');
  private heading = h('h1');
  private tabs = MODES.map((mode) => ({
    mode,
    el: h('button', { role: 'tab', title: `${modeName(mode)} view (${mode[0]})`, on: { click: () => this.setMode(mode) } }, modeName(mode)),
    item: () => ({ label: `${modeName(mode)} view`, hint: mode[0], icon: this.mode === mode ? 'check' : undefined, run: () => this.setMode(mode) }),
    pinned: false,
  }));
  private fitHead: () => void;
  private act: CalendarActions = {
    details: (o) => this.details(o),
    editEvent: (o) => void api.getEvent(o.event.id!).then((full) => this.edit(full), errorToast),
    respond: (o, r) => void this.respond(o.event, r),
    deleteEvent: (o) => void this.deleteEvent(o.event),
    newEvent: (at, allDay) => this.edit(null, at, allDay),
    openDay: (d) => this.openDay(d),
    setVisible: (visible) => void this.setVisible(visible),
  };

  constructor() {
    const head = h('header.cal-head',
      h('button.icon-btn.cal-new', { title: 'New event (n)', 'aria-label': 'New event', on: { click: () => this.edit(null) } }, icon('plus')),
      h('button', { title: 'Go to today (t)', on: { click: () => this.goTo(new Date()) } }, 'Today'),
      h('button.icon-btn', { title: 'Previous (k)', on: { click: () => this.step(-1) } }, icon('chevronLeft')),
      h('button.icon-btn', { title: 'Next (j)', on: { click: () => this.step(1) } }, icon('chevronRight')),
      this.heading,
      h('span.spacer'),
      h('div.segmented', { role: 'tablist' }, this.tabs.map((t) => t.el)));
    const { more, fit } = overflowMenu(head, this.tabs, [this.heading]);
    head.append(more);
    this.fitHead = fit;
    this.root.append(h('section.cal-main', head, h('div.cal-body', h('div.cal-clip', this.track), this.dayPanel)));
    // The time grids scroll together; scroll doesn't bubble, so listen in the
    // capture phase, once, rather than on each redrawn grid.
    this.track.addEventListener('scroll', (e) => {
      const cur = e.target as HTMLElement;
      if (this.placeNext) return;
      if (cur.matches('.time-grid.free')) this.stripScrolled(cur);
      if (!cur.matches('.cal-pane.cur .time-scroll')) return;
      this.hourScroll = cur.scrollTop;
      for (const g of this.track.querySelectorAll<HTMLElement>('.time-scroll')) if (g !== cur) g.scrollTop = cur.scrollTop;
    }, { capture: true, passive: true });
    store.onEvent((e) => {
      if ((e.event === 'calendar_changed' || e.event === 'data_changed') && store.state.view === 'calendar') void this.load();
    });
    // Another part of the app asked to open an event (an alert's "Open").
    store.subscribe((s, changed) => {
      if (changed.has('selectedEvent') && s.selectedEvent && s.view === 'calendar') this.show();
    });
    const s = 'calendar';
    bind('t', 'Today', () => this.goTo(new Date()), s);
    bind('j', 'Next period', () => this.step(1), s);
    bind('k', 'Previous period', () => this.step(-1), s);
    bind('ArrowRight', 'Next period', () => this.step(1), s);
    bind('ArrowLeft', 'Previous period', () => this.step(-1), s);
    bind('d', 'Day view', () => this.setMode('day'), s);
    bind('w', 'Week view', () => this.setMode('week'), s);
    bind('m', 'Month view', () => this.setMode('month'), s);
    bind('a', 'Agenda view', () => this.setMode('agenda'), s);
    bind('n', 'New event', () => this.edit(null), s);
    // The week view scrolls freely instead, and stays wherever it's left.
    swipePager(this.root, () => this.track, (dir) => this.step(dir), () => !isStrip(this.mode));
    lockWheelAxis(this.track, () => this.stripGrid());
    // Places the strip once a grid drawn before it was laid out gets a size,
    // and again whenever its days change width: the scroll position is in
    // pixels, so left alone it would slide to other days (at startup, months
    // past the days drawn, leaving the grid blank).
    new ResizeObserver(() => { if (this.placeNext || this.stripGrid()) this.placeNext = !this.placeScroll(); }).observe(this.track);
  }

  show() {
    this.placeNext = true;
    const wanted = store.state.selectedEvent;
    if (wanted) this.anchor = this.mode === 'week' ? startOfWeek(new Date(wanted.start)) : startOfDay(new Date(wanted.start));
    void this.load();
  }

  private setMode(m: Mode) {
    this.stripFrac = 0;
    this.placeNext = true;
    if (m === 'week' && this.mode !== 'week') {
      this.anchor = startOfWeek(this.anchor);
    } else if (this.mode === 'week' && m !== 'week') {
      // Leaving the week for today if it's in view, rather than its first day.
      const today = startOfDay(new Date());
      if (today >= this.anchor && today < addDays(this.anchor, 7)) this.anchor = today;
    }
    this.mode = m;
    void this.load();
  }

  private openDay(d: Date) {
    this.mode = 'day';
    this.goTo(d);
  }

  /** Show the period holding `d`; the week view starts on its Monday. */
  private goTo(d: Date) {
    this.anchor = this.mode === 'week' ? startOfWeek(d) : startOfDay(d);
    this.stripFrac = 0;
    this.placeNext = true;
    void this.load();
  }

  /** The strip on screen, if the week view is showing. */
  private stripGrid() {
    return isStrip(this.mode) ? this.track.querySelector<HTMLElement>('.time-grid.free') : null;
  }

  private dayWidth(grid: HTMLElement) {
    return grid.querySelector('.day-head')?.getBoundingClientRect().width ?? 0;
  }

  /** Scroll the grids to `hourScroll` and, in the week view, `anchor` +
   * `stripFrac`; false if they aren't laid out yet. */
  private placeScroll(): boolean {
    if (!this.track.clientWidth) return false;
    // Setting a position unchanged would still stop a scroll in flight.
    for (const g of this.track.querySelectorAll<HTMLElement>('.time-scroll')) if (Math.abs(g.scrollTop - this.hourScroll) > 1) g.scrollTop = this.hourScroll;
    const grid = this.stripGrid();
    if (!grid) return true;
    const w = this.dayWidth(grid);
    if (!w) return false;
    this.stripDayWidth = w;
    const left = (daysBetween(this.stripOrigin, this.anchor) + this.stripFrac) * w;
    if (Math.abs(grid.scrollLeft - left) > 1) grid.scrollLeft = left;
    return true;
  }

  /** Track where the user scrolled the strip, drawing more days ahead of it. */
  private stripScrolled(grid: HTMLElement) {
    const w = this.dayWidth(grid);
    // A resize (the sidebar collapsing) scrolls too, but its pixels are still
    // the old width's; the resize observer puts the strip back.
    if (!w || Math.abs(w - this.stripDayWidth) > 0.5) return;
    const pos = grid.scrollLeft / w;
    // The browser rounds scroll positions to the pixel; a hair short of a day is that day.
    const whole = Math.floor(pos + 0.02);
    this.stripFrac = Math.max(0, pos - whole);
    const anchor = addDays(this.stripOrigin, whole);
    if (sameDay(anchor, this.anchor)) return;
    this.anchor = anchor;
    this.heading.textContent = this.title();
    this.fitHead();
    this.renderSide();
    grid.style.setProperty('--allday-rows', String(this.allDayRows()));
    this.drawAhead();
  }

  /** The most all-day events on a day in view (including one partly in view):
   * the week view's all-day row fits those, not every day drawn, or drawing
   * more days would push the hours up and down. */
  private allDayRows() {
    return Math.max(0, ...Array.from({ length: 8 }, (_, i) => this.allDayOn(addDays(this.anchor, i)).length));
  }

  private allDayOn(d: Date) {
    return this.eventsOn(d).filter((o) => o.event.all_day || new Date(o.end).getTime() - new Date(o.start).getTime() >= 86_400_000);
  }

  /** Draw (and load) around the days in view once they drift from what's drawn. */
  private drawAhead() {
    if (this.redrawing || Math.abs(daysBetween(this.stripFrom, this.anchor) - STRIP_DRAW_DAYS) <= STRIP_DRIFT_DAYS) return;
    this.redrawing = true;
    void this.load().finally(() => {
      this.redrawing = false;
      // It may have kept going while that loaded.
      if (isStrip(this.mode)) this.drawAhead();
    });
  }

  /** The anchor `dir` periods away from the current one. */
  private shifted(dir: number): Date {
    const a = this.anchor;
    return this.mode === 'day' ? addDays(a, dir) : this.mode === 'week' ? addDays(a, 7 * dir) : this.mode === 'month' ? new Date(a.getFullYear(), a.getMonth() + dir, 1) : addDays(a, 14 * dir);
  }

  private step(dir: number) {
    this.anchor = this.shifted(dir);
    this.placeNext = true;
    return this.load();
  }

  private range(a = this.anchor): [Date, Date] {
    switch (this.mode) {
      case 'day':
        return [a, addDays(a, 1)];
      case 'week':
        // `a` is the first day in view, which needn't be a Monday.
        return [a, addDays(a, 7)];
      case 'month':
        return monthGrid(a);
      case 'agenda':
        return [a, addDays(a, 30)];
    }
  }

  private async load() {
    const strip = isStrip(this.mode);
    // Lay the strip out afresh around the days in view when it nears an end
    // (or after navigating, which moves the scroll position anyway).
    const at = daysBetween(this.stripOrigin, this.anchor);
    if (strip && (this.placeNext || at < STRIP_DRAW_DAYS || at > STRIP_GRID_DAYS - 7 - STRIP_DRAW_DAYS)) {
      this.stripOrigin = addDays(this.anchor, -STRIP_ORIGIN_DAYS);
      this.placeNext = true;
    }
    const [from, to] = this.loadRange();
    const load = ++this.loads;
    try {
      const [cals, occ] = await Promise.all([api.listCalendars(), api.listEvents(from.toISOString(), to.toISOString())]);
      // A later load has overtaken this one.
      if (load !== this.loads) return;
      this.calendars = cals;
      this.occurrences = occ;
    } catch (e) {
      errorToast(e);
    }
    if (strip) this.stripFrom = from;
    this.render();
    const wanted = store.state.selectedEvent;
    if (wanted) {
      store.set({ selectedEvent: null });
      const o = this.occurrences.find((x) => x.event.id === wanted.id && new Date(x.start).getTime() === new Date(wanted.start).getTime());
      if (o) this.details(o);
      else toast('That event is no longer in your calendar', { kind: 'error' });
    }
  }

  /** What to load: the neighbouring periods too, so they show while swiping. */
  private loadRange(): [Date, Date] {
    const a = this.anchor;
    if (isStrip(this.mode)) return [addDays(a, -STRIP_DRAW_DAYS), addDays(a, 7 + STRIP_DRAW_DAYS)];
    const [from, to] = [this.range(this.shifted(-1))[0], this.range(this.shifted(1))[1]];
    if (this.mode !== 'day') return [from, to];
    // And the sidebar's month, for its dots.
    const [start, end] = monthGrid(a);
    return [start < from ? start : from, end > to ? end : to];
  }

  private title(): string {
    const [from, to] = this.range();
    if (this.mode === 'day') return from.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' });
    if (this.mode === 'month') return this.anchor.toLocaleDateString([], { month: 'long', year: 'numeric' });
    const end = addDays(to, -1);
    return `${from.toLocaleDateString([], { month: 'short', day: 'numeric' })} – ${end.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })}`;
  }

  private render() {
    const view = (a: Date) => (this.mode === 'month' ? this.month(a) : this.mode === 'agenda' ? this.agenda(a) : this.mode === 'day' ? this.timeGrid(1, a) : this.stripView());
    const pane = (dir: number) =>
      // Neighbours are only there to be seen mid-swipe.
      h(`div.cal-pane${dir < 0 ? '.prev' : dir > 0 ? '.next' : '.cur'}`, { inert: dir !== 0 }, view(this.shifted(dir)));
    // The header is built once (its "…" menu holds on to its buttons); only
    // its state changes here.
    this.heading.textContent = this.title();
    for (const t of this.tabs) {
      t.pinned = this.mode === t.mode;
      t.el.classList.toggle('active', t.pinned);
    }
    this.fitHead();
    // Patched, not replaced: this redraws on every sync that touches a
    // calendar, and fresh nodes dropped the hover and blinked the grid.
    this.renderSide();
    this.dayPanel.hidden = this.mode !== 'day';
    if (this.mode === 'day') patch(this.dayPanel, this.dayAgenda());
    if (isStrip(this.mode)) patch(this.track, pane(0));
    else patch(this.track, pane(-1), pane(0), pane(1));
    // Redraws leave the strip where the user has it; only navigating, or
    // showing the view again, moves it.
    if (this.placeNext || !isStrip(this.mode)) this.placeNext = !this.placeScroll();
  }

  private toggleList() {
    this.listOpen = !this.listOpen;
    try {
      localStorage.setItem(LIST_OPEN_KEY, this.listOpen ? '1' : '0');
    } catch {
      /* remembering is a convenience */
    }
    this.renderSide();
  }

  /** The sidebar's month and calendar list. */
  private renderSide() {
    // The day view marks the busy days.
    patch(this.side,
      this.miniMonth(this.mode === 'day'),
      h('div.cal-list',
        h('button.mb-section.mb-toggle', { 'aria-expanded': String(this.listOpen), on: { click: () => this.toggleList() } },
          icon(this.listOpen ? 'chevronDown' : 'chevronRight', 12), 'Calendars'),
        this.listOpen && this.calendars.length === 0 && h('p.muted.small', 'No calendars yet. Calendars sync from accounts that support them.'),
        this.listOpen && this.calendars.map((c) =>
          this.withMenu(h('label.cal-toggle',
            h('input', { type: 'checkbox', checked: c.visible, on: { change: async (e: Event) => { await api.setCalendarVisible(c.id, (e.target as HTMLInputElement).checked); void this.load(); } } }),
            h('span.swatch', { style: { background: c.color } }),
            h('span.cal-name', c.name),
            c.read_only && h('span.cal-lock', { title: 'Read-only: events here can’t be changed', 'aria-label': 'Read-only' }, icon('lock', 14))), () => calendarMenu(this.act, c, this.calendars)))));
  }

  /** The month around `anchor`; `dots` marks the days with events. */
  private miniMonth(dots = false): HTMLElement {
    const a = this.anchor;
    const [start] = monthGrid(a);
    const today = new Date();
    const cells = [];
    for (let i = 0; i < 42; i++) {
      const d = addDays(start, i);
      cells.push(h(`button.mini-day${d.getMonth() !== a.getMonth() ? '.other' : ''}${sameDay(d, today) ? '.today' : ''}${sameDay(d, a) ? '.sel' : ''}${dots && this.eventsOn(d).length ? '.busy' : ''}`, { title: d.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' }), on: { click: () => this.goTo(d) } }, String(d.getDate())));
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

  /** The days drawn, placed in their columns of the strip. */
  private stripView(): HTMLElement {
    return this.timeGrid(7 + 2 * STRIP_DRAW_DAYS, this.stripFrom, daysBetween(this.stripOrigin, this.stripFrom));
  }

  /** `days` days from `from`; in the week strip, starting `at` days into it. */
  private timeGrid(days: number, from: Date, at?: number): HTMLElement {
    const dates = Array.from({ length: days }, (_, i) => addDays(from, i));
    const dayView = this.mode === 'day';
    // Each day keeps its own column, so drawing different days never moves the strip.
    const place = (i: number) => (at === undefined ? {} : { style: { gridColumn: String(at + i + 2) } });
    const today = new Date();
    const timed = (d: Date) => this.eventsOn(d).filter((o) => !o.event.all_day && new Date(o.end).getTime() - new Date(o.start).getTime() < 86_400_000);
    const col = (d: Date, i: number) => {
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
        // The column on screen: after a patch it isn't the one built here.
        const y = e.clientY - (e.currentTarget as HTMLElement).getBoundingClientRect().top;
        return new Date(d.getFullYear(), d.getMonth(), d.getDate(), Math.floor(y / HOUR_PX), y % HOUR_PX >= HOUR_PX / 2 ? 30 : 0);
      };
      const column = h(`div.day-col${sameDay(d, today) ? '.today' : ''}`, {
        ...place(i),
        on: {
          dblclick: (e: MouseEvent) => this.edit(null, slotAt(e)),
        },
      },
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
      return this.withMenu(column, (e) => slotMenu(this.act, slotAt(e), true, dayView));
    };
    const top = h('div.grid-top',
      h('div.grid-head',
        h('div.gutter'),
        dates.map((d, i) => h(`div.day-head${sameDay(d, today) ? '.today' : ''}`, place(i), h('span.dow', d.toLocaleDateString([], { weekday: 'short' })), h('span.dom', String(d.getDate()))))),
      h('div.allday-row', h('div.gutter.muted.small', 'all-day'), dates.map((d, i) => this.withMenu(h('div.allday-cell', place(i), this.allDayOn(d).map((o) => this.chip(o, true))), () => slotMenu(this.act, d, false, dayView)))));
    const body = h('div.time-body',
      h('div.gutter.hours', Array.from({ length: 24 }, (_, hr) => h('div.hour-label', { style: { top: `${hr * HOUR_PX}px` } }, hr === 0 ? '' : new Date(2000, 0, 1, hr).toLocaleTimeString([], { hour: 'numeric' })))),
      dates.map(col));
    // A day scrolls only up and down (it pages sideways); the week strip
    // scrolls both ways, with the header and hours stuck to its edges.
    if (at === undefined) return h('div.time-grid.days-1', top, h('div.time-scroll', body));
    return h('div.time-grid.free.time-scroll', { style: `--days: ${STRIP_GRID_DAYS}; --allday-rows: ${this.allDayRows()}` }, top, body);
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
          h('button.month-date', { title: 'Open this day', on: { click: () => this.openDay(d) } }, String(d.getDate())),
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
        evs.map((o) => this.agendaItem(o))));
    }
    return h('div.agenda', days.length ? days : h('div.empty-state', icon('calendar', 36), h('p', 'No events in the next 30 days.')));
  }

  private agendaItem(o: Occurrence): HTMLElement {
    return this.withMenu(h('button.agenda-item', { on: { click: () => this.details(o) } },
      h('span.swatch', { style: { background: o.color } }),
      h('span.agenda-time', timeRange(o.start, o.end, o.event.all_day)),
      h('span.agenda-title', o.event.title),
      o.event.location && h('span.muted.small', o.event.location),
      o.event.my_status === 'needs_action' && h('span.status-chip.pending', 'needs response')), o);
  }

  /** The day view's list of the day's events, all-day ones first. */
  private dayAgenda(): HTMLElement {
    const d = this.anchor;
    const evs = this.eventsOn(d).sort((a, b) => Number(b.event.all_day) - Number(a.event.all_day) || a.start.localeCompare(b.start));
    return h('section.day-agenda',
      h('h3', d.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })),
      evs.length ? evs.map((o) => this.agendaItem(o)) : h('p.muted.small', 'No events.'));
  }

  private details(o: Occurrence) {
    const e = o.event;
    const cal = this.calendars.find((c) => c.id === e.calendar_id);
    const respond = async (r: PartStat) => {
      if (await this.respond(e, r)) m.close();
    };
    const m = modal(e.title, h('div.event-details',
      h('div', icon('clock', 16), ` ${new Date(o.start).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })} · ${timeRange(o.start, o.end, e.all_day)}`),
      e.rrule && h('div.muted.small', eventRepeatText(e)),
      e.location && h('div', icon('location', 16), ` ${e.location}`),
      !!e.alerts?.length && h('div.small', icon('bell', 16), ` ${e.alerts.map((a) => alertLabel(a.minutes_before, e.all_day)).join(', ')}`),
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
