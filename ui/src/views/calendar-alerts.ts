// The event editor's alerts: a row per alert, each a choice of when, and
// "Add alert" (0056).

import { ALL_DAY_ALERTS, TIMED_ALERTS, alertLabel } from '../lib/alerts';
import { h, icon, patch } from '../lib/dom';
import type { Alert } from '../types';

/** Edits a list of alerts. `allDay` says which kind of event they're for;
 *  call `refresh` when that changes. */
export function alertsPicker(initial: Alert[], allDay: () => boolean, defaults: (allDay: boolean) => Alert[]) {
  let alerts = initial.map((a) => a.minutes_before);
  // Until the user picks something, switching to or from all day starts the
  // new kind of event with its own default.
  let touched = false;
  let wasAllDay = allDay();
  const el = h('div.ev-alerts');

  const choices = (current: number) => {
    const all = allDay();
    const offered = all ? ALL_DAY_ALERTS : TIMED_ALERTS;
    // Keep an alert set elsewhere that isn't one of ours.
    const list = offered.includes(current) ? offered : [...offered, current].sort((a, b) => a - b);
    return list.map((m) => h('option', { value: String(m), selected: m === current }, alertLabel(m, all)));
  };
  const render = () => {
    patch(el,
      ...alerts.map((m, i) => h('div.ev-alert',
        h('select', { 'aria-label': 'Alert', on: { change: (e: Event) => { touched = true; alerts[i] = Number((e.currentTarget as HTMLSelectElement).value); render(); } } }, choices(m)),
        h('button.icon-btn', { type: 'button', title: 'Remove this alert', 'aria-label': 'Remove alert', on: { click: () => { touched = true; alerts.splice(i, 1); render(); } } }, icon('close', 14)))),
      h('button.link', { type: 'button', title: 'Add another alert', on: { click: add } }, alerts.length ? 'Add another alert' : 'Add alert'));
  };
  const add = () => {
    touched = true;
    const offered = allDay() ? ALL_DAY_ALERTS : TIMED_ALERTS;
    alerts.push(offered.find((m) => !alerts.includes(m) && (allDay() || m >= 10)) ?? offered[0]);
    render();
    el.querySelectorAll('select')[alerts.length - 1]?.focus();
  };
  render();
  return {
    el,
    value: (): Alert[] => [...new Set(alerts)].map((minutes_before) => ({ minutes_before })),
    refresh() {
      if (allDay() !== wasAllDay) {
        wasAllDay = allDay();
        if (!touched) alerts = defaults(wasAllDay).map((a) => a.minutes_before);
      }
      render();
    },
  };
}
