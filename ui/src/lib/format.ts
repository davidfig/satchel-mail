import type { Address } from '../types';

export function displayName(a: Address | null | undefined): string {
  if (!a) return '(unknown)';
  return a.name?.trim() || a.email;
}

export function shortName(a: Address | null | undefined): string {
  const n = displayName(a);
  if (n.includes('@')) return n.split('@')[0];
  return n.split(/\s+/)[0];
}

export function formatAddress(a: Address): string {
  return a.name ? `${a.name} <${a.email}>` : a.email;
}

export function participantsLabel(list: Address[], me: Set<string>): string {
  if (list.length === 1) return me.has(list[0].email.toLowerCase()) ? 'me' : displayName(list[0]);
  const names = list.map((a) => (me.has(a.email.toLowerCase()) ? 'me' : shortName(a)));
  if (names.length <= 3) return names.join(', ');
  return `${names.slice(0, 2).join(', ')} +${names.length - 2}`;
}

const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** Compact list date: time today, weekday this week, date otherwise. */
export function listDate(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  if (sameDay(d, now)) return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  const diff = (now.getTime() - d.getTime()) / 86_400_000;
  if (diff < 6) return d.toLocaleDateString([], { weekday: 'short' });
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
  return d.toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' });
}

export function fullDate(iso: string): string {
  return new Date(iso).toLocaleString([], { weekday: 'short', year: 'numeric', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

export function timeRange(startIso: string, endIso: string, allDay: boolean): string {
  if (allDay) return 'All day';
  const s = new Date(startIso);
  const e = new Date(endIso);
  const t = (d: Date) => d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  return `${t(s)} – ${t(e)}`;
}

export function humanSize(n: number): string {
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

export function relative(iso: string | null): string {
  if (!iso) return 'never';
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

/** "Name <a@b>, c@d" -> Address[] */
export function parseAddresses(s: string): Address[] {
  const out: Address[] = [];
  for (const part of s.split(/[,;\n]/)) {
    const p = part.trim();
    if (!p) continue;
    const m = p.match(/^(.*)<([^>]+)>$/);
    if (m) out.push({ name: m[1].trim().replace(/^"|"$/g, '') || null, email: m[2].trim() });
    else out.push({ email: p });
  }
  return out;
}

export function initials(a: Address | null | undefined): string {
  const n = displayName(a);
  const parts = n.replace(/[<>"@.]/g, ' ').trim().split(/\s+/);
  return ((parts[0]?.[0] ?? '?') + (parts[1]?.[0] ?? '')).toUpperCase();
}

/** Stable pastel-ish color per address for avatars. */
export function avatarColor(email: string): string {
  let h = 0;
  for (const c of email.toLowerCase()) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return `hsl(${h % 360} 55% 48%)`;
}
