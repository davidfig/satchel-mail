// The event editor's repeat picker: presets worded from the start date
// ("Every month on the last Friday"), and a Custom… panel for the interval,
// weekdays, monthly day and end. The panel is a form holding its own fields,
// so it toggles `hidden` rather than redrawing.

import { h, listen, replace } from '../lib/dom';
import { DAYS, type Day, type Rule, dayOf, describeRule, formatRule, nthOf, parseRule, untilDate } from '../lib/rrule';
import { fromWall, toWall } from '../lib/tz';

/** Where the series starts, read from the editor when needed. */
export interface RepeatStart {
  date: string;
  time: string;
  zone: string;
  allDay: boolean;
}

type Ends = 'never' | 'on' | 'after';
const UNITS: [Rule['freq'], string][] = [['DAILY', 'day'], ['WEEKLY', 'week'], ['MONTHLY', 'month'], ['YEARLY', 'year']];

export function repeatPicker(initial: string | null, start: () => RepeatStart) {
  const startDay = () => {
    const [y, m, d] = start().date.split('-').map(Number);
    return y ? new Date(y, m - 1, d) : new Date();
  };
  const nthRule = (s: Date) => {
    const { n, last } = nthOf(s);
    return `FREQ=MONTHLY;BYDAY=${last ? -1 : n}${dayOf(s)}`;
  };
  // Each preset's rule, worked out from the start so it follows date changes.
  const presets = (s: Date): [string, string | null][] => [
    ['none', null],
    ['daily', 'FREQ=DAILY'],
    ['weekly', 'FREQ=WEEKLY'],
    ['weekdays', 'FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR'],
    ['monthly', 'FREQ=MONTHLY'],
    ['monthly-nth', nthRule(s)],
    ['yearly', 'FREQ=YEARLY'],
  ];
  const presetLabel = (key: string, rule: string | null, s: Date) =>
    key === 'none' ? 'Does not repeat' : key === 'weekdays' ? 'Every weekday (Monday to Friday)' : describeRule(parseRule(rule!)!, s);

  // The rule the panel edits, and how it ends.
  const parsed = initial ? parseRule(initial) : null;
  let rule: Rule = parsed ?? { freq: 'WEEKLY', interval: 1, days: [dayOf(startDay())], rest: [] };
  let ends: Ends = rule.count ? 'after' : rule.until ? 'on' : 'never';
  const preset = presets(startDay()).find(([, r]) => r === (initial ?? null))?.[0];
  // A rule the panel can't show is kept as it is unless another is picked.
  const keepRaw = !!initial && !preset && !parsed;

  const select = h('select', { 'aria-label': 'Repeat' });
  const interval = h('input', { type: 'number', min: 1, max: 99, value: String(rule.interval), 'aria-label': 'Repeat every' });
  const unit = h('select', { 'aria-label': 'Repeat unit' }, UNITS.map(([v]) => h('option', { value: v, selected: v === rule.freq })));
  const chips = DAYS.map((d) =>
    h('button.day-chip', { type: 'button', title: new Date(2024, 0, 1 + DAYS.indexOf(d)).toLocaleDateString('en-US', { weekday: 'long' }), dataset: { day: d }, on: { click: () => toggleDay(d) } }, d[0]));
  const monthly = h('select', { 'aria-label': 'Day of the month' });
  const endsSel = h('select', { 'aria-label': 'Ends' }, [['never', 'Never'], ['on', 'On a date'], ['after', 'After']].map(([v, l]) => h('option', { value: v, selected: v === ends }, l)));
  const zone0 = start().zone;
  const until = h('input', { type: 'date', 'aria-label': 'Last date', value: rule.until ? untilDate(rule.until, (utc) => toWall(utc, start().allDay ? 'UTC' : zone0).date) : '' });
  const count = h('input', { type: 'number', min: 1, max: 999, value: String(rule.count ?? 10), 'aria-label': 'Number of times' });
  const times = h('span.muted', 'times');
  const summary = h('div.muted.small');
  const weekRow = h('div.repeat-row', h('span.muted', 'On'), h('div.day-chips', chips));
  const monthRow = h('div.repeat-row', monthly);
  const panel = h('div.repeat-panel',
    h('div.repeat-row', h('span.muted', 'Every'), interval, unit),
    weekRow,
    monthRow,
    h('div.repeat-row', h('span.muted', 'Ends'), endsSel, until, count, times),
    summary);

  const toggleDay = (d: Day) => {
    const on = rule.days.includes(d);
    if (on && rule.days.length === 1) return;
    rule.days = on ? rule.days.filter((x) => x !== d) : [...rule.days, d];
    refresh();
  };

  const custom = (): Rule => {
    const r: Rule = { ...rule, count: undefined, until: undefined };
    if (ends === 'after') r.count = Math.max(1, Number(count.value) || 1);
    if (ends === 'on' && until.value) {
      const s = start();
      r.until = s.allDay
        ? until.value.replace(/-/g, '')
        : fromWall(until.value, s.time || '00:00', s.zone).toISOString().replace(/[-:]/g, '').replace(/\.\d+/, '');
    }
    return r;
  };

  /** Relabel everything for the current start date and choices. */
  const refresh = () => {
    const s = startDay();
    const key = select.value || (keepRaw ? 'raw' : preset ?? (initial ? 'custom' : 'none'));
    const options = presets(s).map(([k, r]) => h('option', { value: k, selected: k === key }, presetLabel(k, r, s)));
    replace(select, options,
      keepRaw ? h('option', { value: 'raw', selected: key === 'raw' }, `Custom rule (${initial})`) : null,
      h('option', { value: 'custom', selected: key === 'custom' }, 'Custom…'));
    select.value = key;
    panel.hidden = key !== 'custom';

    for (const o of unit.options) o.textContent = UNITS.find(([v]) => v === o.value)![1] + (rule.interval === 1 ? '' : 's');
    weekRow.hidden = rule.freq !== 'WEEKLY';
    for (const c of chips) c.setAttribute('aria-pressed', String(rule.days.includes(c.dataset.day as Day)));
    monthRow.hidden = rule.freq !== 'MONTHLY';
    // "Every month on the last Friday" → "On the last Friday".
    const on = (r: Rule) => describeRule(r, s).replace(/^Every .*? on /, 'On ');
    const nth = parseRule(nthRule(s))!;
    replace(monthly,
      h('option', { value: 'day', selected: !rule.nth }, on({ ...nth, nth: undefined, monthDay: rule.monthDay })),
      h('option', { value: 'nth', selected: !!rule.nth }, on(nth)));
    if (rule.nth) rule.nth = nth.nth;
    until.hidden = ends !== 'on';
    count.hidden = times.hidden = ends !== 'after';
    const r = custom();
    summary.textContent = describeRule(r, s, ends === 'on' && until.value ? until.value : undefined);
  };

  let lastKey = '';
  listen(select, 'change', () => {
    // Custom… starts from whichever preset was showing.
    if (select.value === 'custom' && lastKey !== 'custom') {
      const from = presets(startDay()).find(([k]) => k === lastKey)?.[1];
      const p = from ? parseRule(from) : null;
      if (p) rule = { ...p, days: p.freq === 'WEEKLY' && !p.days.length ? [dayOf(startDay())] : p.days };
      interval.value = String(rule.interval);
      unit.value = rule.freq;
    }
    lastKey = select.value;
    refresh();
  });
  listen(interval, 'input', () => { rule.interval = Math.max(1, Math.min(99, Number(interval.value) || 1)); refresh(); });
  listen(unit, 'change', () => {
    rule.freq = unit.value as Rule['freq'];
    if (rule.freq === 'WEEKLY' && !rule.days.length) rule.days = [dayOf(startDay())];
    refresh();
  });
  listen(monthly, 'change', () => {
    rule.monthDay = undefined;
    rule.nth = monthly.value === 'nth' ? parseRule(nthRule(startDay()))!.nth : undefined;
    refresh();
  });
  listen(endsSel, 'change', () => {
    ends = endsSel.value as Ends;
    if (ends === 'on' && !until.value) {
      // Three months out to begin with.
      const s = startDay();
      const d = new Date(s.getFullYear(), s.getMonth() + 3, s.getDate());
      until.value = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    }
    refresh();
  });
  listen(until, 'change', refresh);
  listen(count, 'input', refresh);
  refresh();
  lastKey = select.value;

  return {
    el: h('div.repeat', select, panel),
    /** Call when the start changes: labels and "last Friday"-style rules follow it. */
    refresh,
    /** The RRULE to save, or null. */
    value(): string | null {
      const key = select.value;
      if (key === 'raw') return initial;
      if (key === 'custom') return formatRule(custom());
      return presets(startDay()).find(([k]) => k === key)?.[1] ?? null;
    },
    /** A problem to fix before saving, if any. */
    problem(): string | null {
      if (select.value !== 'custom' || ends !== 'on') return null;
      if (!until.value) return 'Pick the date the event stops repeating';
      return until.value < start().date ? 'The repeat ends before the event starts' : null;
    },
  };
}
