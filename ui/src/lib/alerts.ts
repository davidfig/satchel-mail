// Calendar alerts in words, and the choices offered for them (0056).

const DAY = 1440;

/** "10 minutes", "1 hour", "1 hour 30 minutes", "2 days", "1 week". */
function span(minutes: number): string {
  const unit = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
  if (minutes % (7 * DAY) === 0) return unit(minutes / (7 * DAY), 'week');
  if (minutes % DAY === 0) return unit(minutes / DAY, 'day');
  if (minutes % 60 === 0) return unit(minutes / 60, 'hour');
  if (minutes > 60) return `${unit(Math.floor(minutes / 60), 'hour')} ${unit(minutes % 60, 'minute')}`;
  return unit(minutes, 'minute');
}

/** A clock time from minutes after midnight: "9 AM", "8:30 AM". */
function clock(minutes: number): string {
  const d = new Date(2000, 0, 1, Math.floor(minutes / 60), minutes % 60);
  return d.toLocaleTimeString([], { hour: 'numeric', minute: minutes % 60 ? '2-digit' : undefined });
}

/** An alert in words: "10 minutes before", "At start time", "1 day before at 9 AM". */
export function alertLabel(minutesBefore: number, allDay: boolean): string {
  if (allDay) {
    // Counted from midnight on the event's first day.
    const days = Math.max(0, Math.ceil(minutesBefore / DAY));
    const at = clock(days * DAY - minutesBefore);
    if (days === 0) return `On the day at ${at}`;
    return `${days === 1 ? '1 day' : days % 7 === 0 ? span(days * DAY) : `${days} days`} before at ${at}`;
  }
  if (minutesBefore === 0) return 'At start time';
  return minutesBefore > 0 ? `${span(minutesBefore)} before` : `${span(-minutesBefore)} after start`;
}

/** The alerts offered for timed events, in minutes before. */
export const TIMED_ALERTS = [0, 5, 10, 15, 30, 60, 120, DAY, 2 * DAY, 7 * DAY];

/** The alerts offered for all-day events: 9 AM on the day, or days before. */
export const ALL_DAY_ALERTS = [-540, DAY - 540, 2 * DAY - 540, 7 * DAY - 540];
