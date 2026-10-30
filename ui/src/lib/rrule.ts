// Repeat rules (RFC 5545 RRULE): parsing the parts the editor understands,
// writing them back, and describing them in words. Parts we don't edit
// (BYSETPOS, WKST, …) are kept as they are.

export type Freq = 'DAILY' | 'WEEKLY' | 'MONTHLY' | 'YEARLY';
export const DAYS = ['MO', 'TU', 'WE', 'TH', 'FR', 'SA', 'SU'] as const;
export type Day = (typeof DAYS)[number];

export interface Rule {
  freq: Freq;
  interval: number;
  /** Weekly: the days it falls on (empty = the start's day). */
  days: Day[];
  /** Monthly: "the 2nd Tuesday" (n = -1 for the last). */
  nth?: { n: number; day: Day };
  /** Monthly: the day of the month (otherwise the start's). */
  monthDay?: number;
  count?: number;
  /** Raw UNTIL: YYYYMMDD or YYYYMMDDTHHMMSSZ. */
  until?: string;
  /** Parts we keep but don't edit. */
  rest: string[];
}

const FREQS: Freq[] = ['DAILY', 'WEEKLY', 'MONTHLY', 'YEARLY'];

/** null for rules the editor can't show (hourly, several BYDAY ordinals, …). */
export function parseRule(text: string): Rule | null {
  const r: Rule = { freq: 'DAILY', interval: 1, days: [], rest: [] };
  let freq: string | undefined;
  for (const part of text.replace(/^RRULE:/i, '').split(';').filter(Boolean)) {
    const [k, v = ''] = part.split('=');
    const key = k.toUpperCase();
    if (key === 'FREQ') freq = v.toUpperCase();
    else if (key === 'INTERVAL') r.interval = Math.max(1, Number(v) || 1);
    else if (key === 'COUNT') r.count = Number(v) || undefined;
    else if (key === 'UNTIL') r.until = v;
    else if (key === 'BYMONTHDAY' && /^\d+$/.test(v)) r.monthDay = Number(v);
    else if (key === 'BYDAY') {
      const items = v.toUpperCase().split(',');
      const m = items.length === 1 && items[0].match(/^([+-]?\d)(MO|TU|WE|TH|FR|SA|SU)$/);
      if (m) r.nth = { n: Number(m[1]), day: m[2] as Day };
      else if (items.every((d) => (DAYS as readonly string[]).includes(d))) r.days = items as Day[];
      else return null;
    } else r.rest.push(part);
  }
  if (!FREQS.includes(freq as Freq)) return null;
  r.freq = freq as Freq;
  return r;
}

export function formatRule(r: Rule): string {
  const parts = [`FREQ=${r.freq}`];
  if (r.interval > 1) parts.push(`INTERVAL=${r.interval}`);
  if (r.freq === 'WEEKLY' && r.days.length) parts.push(`BYDAY=${DAYS.filter((d) => r.days.includes(d)).join(',')}`);
  if (r.freq === 'MONTHLY' && r.nth) parts.push(`BYDAY=${r.nth.n}${r.nth.day}`);
  if (r.freq === 'MONTHLY' && !r.nth && r.monthDay) parts.push(`BYMONTHDAY=${r.monthDay}`);
  if (r.count) parts.push(`COUNT=${r.count}`);
  else if (r.until) parts.push(`UNTIL=${r.until}`);
  return [...parts, ...r.rest].join(';');
}

/** The start's weekday as an RRULE day. `start` is a wall-clock date. */
export const dayOf = (start: Date): Day => DAYS[(start.getDay() + 6) % 7];

/** Which `day` of its month `start` is: 1–4, or -1 when it's in the last week. */
export function nthOf(start: Date): { n: number; last: boolean } {
  const len = new Date(start.getFullYear(), start.getMonth() + 1, 0).getDate();
  return { n: Math.ceil(start.getDate() / 7), last: start.getDate() + 7 > len };
}

const dayName = (d: Day, width: 'long' | 'short' = 'long') => new Date(2024, 0, 1 + DAYS.indexOf(d)).toLocaleDateString('en-US', { weekday: width });
const ORDINAL: Record<string, string> = { '1': 'first', '2': 'second', '3': 'third', '4': 'fourth', '5': 'fifth', '-1': 'last' };
const list = (items: string[]) => new Intl.ListFormat('en-US', { type: 'conjunction' }).format(items);
const every = (n: number, unit: string) => (n === 1 ? `Every ${unit}` : `Every ${n} ${unit}s`);

/** The UNTIL value as a wall-clock date (local fields), given how to read a UTC one. */
export function untilDate(until: string, utcToDate: (utc: Date) => string): string {
  const m = until.match(/^(\d{4})(\d{2})(\d{2})(?:T(\d{2})(\d{2})(\d{2})(Z?))?$/);
  if (!m) return '';
  if (!m[4] || m[7] !== 'Z') return `${m[1]}-${m[2]}-${m[3]}`;
  return utcToDate(new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6])));
}

/** "Every 2 weeks on Monday and Wednesday, until Dec 31, 2026". `start` is
 *  the series' first day (wall clock); `until` is the end date, if any. */
export function describeRule(r: Rule, start: Date, until?: string): string {
  let s: string;
  switch (r.freq) {
    case 'DAILY':
      s = every(r.interval, 'day');
      break;
    case 'WEEKLY': {
      const days = r.days.length ? DAYS.filter((d) => r.days.includes(d)) : [dayOf(start)];
      if (r.interval === 1 && days.join() === 'MO,TU,WE,TH,FR') s = 'Every weekday';
      else if (r.interval === 1 && days.length === 7) s = 'Every day';
      else s = `${every(r.interval, 'week')} on ${list(days.map((d) => dayName(d, days.length > 2 ? 'short' : 'long')))}`;
      break;
    }
    case 'MONTHLY':
      s = r.nth
        ? `${every(r.interval, 'month')} on the ${ORDINAL[r.nth.n] ?? `${r.nth.n}th`} ${dayName(r.nth.day)}`
        : `${every(r.interval, 'month')} on day ${r.monthDay ?? start.getDate()}`;
      break;
    case 'YEARLY':
      s = `${every(r.interval, 'year')} on ${start.toLocaleDateString('en-US', { month: 'long', day: 'numeric' })}`;
      break;
  }
  if (r.count) s += r.count === 1 ? ', once' : `, ${r.count} times`;
  else if (until) {
    const [y, m, d] = until.split('-').map(Number);
    s += `, until ${new Date(y, m - 1, d).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}`;
  }
  return s;
}
