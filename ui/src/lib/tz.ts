// Time zones: converting between an instant and the wall-clock date and time
// in an IANA zone, using Intl (the runtime's tz database, not one of ours).

export const localZone = Intl.DateTimeFormat().resolvedOptions().timeZone;

/** Every IANA zone the runtime knows, plus UTC. */
export function allZones(): string[] {
  const list = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone') ?? [localZone];
  return list.includes('UTC') ? list : [...list, 'UTC'];
}

const formatters = new Map<string, Intl.DateTimeFormat>();
function parts(tz: string, at: Date) {
  let f = formatters.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric' });
    formatters.set(tz, f);
  }
  const n: Record<string, number> = {};
  for (const p of f.formatToParts(at)) if (p.type !== 'literal') n[p.type] = Number(p.value);
  return n;
}

/** Minutes `tz` is ahead of UTC at `at`. */
export function offsetMinutes(tz: string, at: Date): number {
  const n = parts(tz, at);
  const asUtc = Date.UTC(n.year, n.month - 1, n.day, n.hour, n.minute, n.second);
  return Math.round((asUtc - Math.floor(at.getTime() / 1000) * 1000) / 60_000);
}

const pad = (n: number) => String(n).padStart(2, '0');

/** The wall-clock date (YYYY-MM-DD) and time (HH:MM) of `at` in `tz`. */
export function toWall(at: Date, tz: string): { date: string; time: string } {
  const d = new Date(at.getTime() + offsetMinutes(tz, at) * 60_000);
  return { date: `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`, time: `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}` };
}

/** The instant a wall-clock date and time in `tz` names. A time skipped by a
 *  DST jump lands just after it; a repeated one takes the first. */
export function fromWall(date: string, time: string, tz: string): Date {
  const [y, mo, d] = date.split('-').map(Number);
  const [hh, mm] = (time || '00:00').split(':').map(Number);
  const wall = Date.UTC(y, mo - 1, d, hh, mm);
  const DAY = 86_400_000;
  // Offsets either side of any transition near this time.
  const before = offsetMinutes(tz, new Date(wall - DAY)) * 60_000;
  const after = offsetMinutes(tz, new Date(wall + DAY)) * 60_000;
  const fits = (t: number) => t + offsetMinutes(tz, new Date(t)) * 60_000 === wall;
  const valid = [wall - before, wall - after].filter(fits);
  return new Date(valid.length ? Math.min(...valid) : wall - before);
}

/** "GMT-7", "GMT+5:30", "GMT". */
export function offsetLabel(tz: string, at = new Date()): string {
  const m = offsetMinutes(tz, at);
  if (!m) return 'GMT';
  const a = Math.abs(m);
  return `GMT${m < 0 ? '-' : '+'}${Math.floor(a / 60)}${a % 60 ? `:${pad(a % 60)}` : ''}`;
}

/** "America/Los_Angeles" → "Los Angeles". */
export const zoneCity = (tz: string) => tz.split('/').pop()!.replace(/_/g, ' ');

/** The zone's long name in the user's language ("Pacific Daylight Time"), for searching. */
export function zoneLongName(tz: string, at = new Date()): string {
  return new Intl.DateTimeFormat([], { timeZone: tz, timeZoneName: 'long' }).formatToParts(at).find((p) => p.type === 'timeZoneName')?.value ?? '';
}
