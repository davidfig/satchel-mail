// An event's repeat rule in words, read in the event's own time zone.

import { describeRule, parseRule, untilDate } from './rrule';
import { localZone, toWall } from './tz';

/** An event's repeat in words, or the raw rule when we can't read it. */
export function eventRepeatText(e: { rrule?: string | null; start: string; all_day: boolean; timezone?: string | null }): string | null {
  if (!e.rrule) return null;
  const rule = parseRule(e.rrule);
  if (!rule) return `Repeats (${e.rrule})`;
  const zone = e.all_day ? 'UTC' : e.timezone ?? localZone;
  const [y, m, d] = toWall(new Date(e.start), zone).date.split('-').map(Number);
  return describeRule(rule, new Date(y, m - 1, d), rule.until ? untilDate(rule.until, (utc) => toWall(utc, zone).date) : undefined);
}
