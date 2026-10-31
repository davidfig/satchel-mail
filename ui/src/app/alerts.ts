// Calendar alerts that have gone off: a stack of cards with Snooze, Open and
// Dismiss, and a system notification while the app is in the background.
// The core decides when an alert is due (see `CoreEvent::AlertsChanged`, 0056).

import { api } from '../api';
import { alertLabel } from '../lib/alerts';
import { h, icon, patch } from '../lib/dom';
import { timeRange } from '../lib/format';
import { errorToast, popupMenu } from '../lib/ui';
import type { MenuItem } from '../lib/ui';
import { store } from '../state';
import { transport } from '../transport';
import type { CalendarAlert } from '../types';

const MINUTE = 60_000;

/** The day of an occurrence: "Today", "Tomorrow", "Mon, Nov 3". All-day
 *  events are UTC midnights, so their day is read in UTC. */
function day(a: CalendarAlert): string {
  const zone = a.all_day ? 'UTC' : undefined;
  const key = (d: Date) => d.toLocaleDateString('en-CA', { timeZone: zone });
  const s = new Date(a.start);
  const today = new Date();
  if (key(s) === key(today)) return 'Today';
  if (key(s) === key(new Date(today.getTime() + 86_400_000))) return 'Tomorrow';
  return s.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric', timeZone: zone });
}

/** When the event is, from now: "In 10 min · 2:00 – 3:00 PM", "Started 5 min ago". */
function when(a: CalendarAlert): string {
  const range = timeRange(a.start, a.end, a.all_day);
  if (a.all_day) return `${day(a)} · ${range}`;
  const mins = Math.round((new Date(a.start).getTime() - Date.now()) / MINUTE);
  const ended = new Date(a.end).getTime() <= Date.now();
  if (mins > 0 && mins < 60) return `In ${mins} min · ${range}`;
  if (mins === 0) return `Now · ${range}`;
  if (mins < 0 && mins > -60 && !ended) return `Started ${-mins} min ago · ${range}`;
  return `${day(a)} · ${range}`;
}

/** Shows the open alerts. Install once, after the store is ready. */
export function installAlerts() {
  const stack = h('div.alert-stack', { role: 'region', 'aria-label': 'Calendar alerts', 'aria-live': 'polite' });
  document.body.append(stack);
  let alerts: CalendarAlert[] = [];

  const act = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
    } catch (e) {
      errorToast(e);
    }
    void load();
  };
  const snooze = (a: CalendarAlert, until: Date) => void act(() => api.snoozeAlert(a.id, until.toISOString()));
  const snoozeMenu = (a: CalendarAlert, anchor: HTMLElement) => {
    const now = Date.now();
    const start = new Date(a.start).getTime();
    const after = (m: number) => new Date(now + m * MINUTE);
    const items: (MenuItem | '-')[] = [
      { label: '5 minutes', run: () => snooze(a, after(5)) },
      { label: '10 minutes', run: () => snooze(a, after(10)) },
      { label: '15 minutes', run: () => snooze(a, after(15)) },
      { label: '1 hour', run: () => snooze(a, after(60)) },
    ];
    const before = start - 5 * MINUTE;
    if (!a.all_day && before > now + MINUTE) items.push('-', { label: '5 minutes before it starts', icon: 'clock', run: () => snooze(a, new Date(before)) });
    if (!a.all_day && start > now + MINUTE) items.push({ label: 'When it starts', icon: 'clock', run: () => snooze(a, new Date(start)) });
    popupMenu(anchor, items);
  };
  const open = (a: CalendarAlert) => store.set({ view: 'calendar', selectedEvent: { id: a.event_id, start: a.start } });

  const card = (a: CalendarAlert) => h('div.alert-card', { style: { '--ev': a.color } as unknown as Partial<CSSStyleDeclaration> },
    h('div.alert-head',
      icon('bell', 16),
      h('span.alert-title', a.title || '(no title)'),
      h('button.icon-btn', { title: 'Dismiss', 'aria-label': 'Dismiss', on: { click: () => void act(() => api.dismissAlert(a.id)) } }, icon('close', 14))),
    h('div.alert-when', when(a)),
    a.location && h('div.alert-where.muted', a.location),
    h('div.alert-actions',
      h('span.muted.small', alertLabel(a.minutes_before, a.all_day)),
      h('span.spacer'),
      h('button', { title: 'Remind me again later', on: { click: (e: MouseEvent) => snoozeMenu(a, e.currentTarget as HTMLElement) } }, 'Snooze', icon('chevronDown', 14)),
      h('button', { title: 'Open the event, to see it or change its alerts', on: { click: () => open(a) } }, 'Open')));

  const render = () => {
    const showing = alerts.filter((a) => !a.snoozed);
    stack.hidden = !showing.length;
    patch(stack,
      showing.length > 1 && h('div.alert-all', h('span.muted.small', `${showing.length} alerts`), h('button.link', { title: 'Dismiss every alert', on: { click: () => void act(() => api.dismissAlert(null)) } }, 'Dismiss all')),
      ...showing.map(card));
  };
  const load = async () => {
    try {
      alerts = await api.listAlerts();
    } catch {
      return;
    }
    render();
  };

  // The Mac app schedules alerts with the system, which shows them (0057).
  // Elsewhere the first alert is when to ask about notifications.
  const notify = async (due: CalendarAlert[]) => {
    if (!due.length || document.hasFocus() || (await transport.osAlerts()) || !(await transport.canNotify(true))) return;
    for (const a of due.slice(0, 3)) {
      transport.notify(a.title || '(no title)', [when(a), a.location].filter(Boolean).join('\n'), () => open(a));
    }
  };
  store.onEvent((e) => {
    if (e.event === 'alerts_changed') void notify(e.due);
    // An edit can change an alert's title or time.
    if (e.event === 'alerts_changed' || e.event === 'calendar_changed' || e.event === 'data_changed') void load();
  });
  transport.onOpenEvent((e) => store.set({ view: 'calendar', selectedEvent: e }));
  // "In 10 min" counts down.
  setInterval(() => { if (alerts.length) render(); }, 30_000);
  void load();
}
