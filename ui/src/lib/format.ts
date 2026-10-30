import type { Address } from '../types';

export function displayName(a: Address | null | undefined): string {
  if (!a) return '(unknown)';
  return a.name?.trim() || a.email;
}

/** The part of an address after the @, for showing where mail really comes from. */
export function domainOf(a: Address | null | undefined): string {
  return a?.email.split('@').pop()?.toLowerCase() ?? '';
}

export function shortName(a: Address | null | undefined): string {
  const n = displayName(a);
  if (n.includes('@')) return n.split('@')[0];
  // "Doe, John" (common in Outlook) is first name John.
  const [last, first] = n.split(',', 2).map((s) => s.trim());
  if (first) return first.split(/\s+/)[0];
  return last.split(/\s+/)[0];
}

export function formatAddress(a: Address): string {
  if (!a.name) return a.email;
  // Quote names with list or address punctuation ("Doe, John") so they read
  // back as one address.
  const name = /[,;<>"@()]/.test(a.name) ? `"${a.name.replace(/["\\]/g, '\\$&')}"` : a.name;
  return `${name} <${a.email}>`;
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

/** `relative` for tight spaces: "now", "5m", "3h", "2d". */
export function relativeShort(iso: string | null): string {
  if (!iso) return 'never';
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** Split an address list on commas, semicolons and newlines, except inside
 *  quotes or angle brackets. A piece with no address in it followed by a
 *  "Name <address>" piece is a display name that was split at a comma
 *  (Doe, John <john@x>), so they join. */
function splitAddressList(s: string): string[] {
  const parts: string[] = [];
  let cur = '';
  let quoted = false;
  let angle = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quoted && c === '\\' && i + 1 < s.length) {
      cur += c + s[++i];
      continue;
    }
    if (c === '"' && !angle) quoted = !quoted;
    else if (c === '<' && !quoted) angle = true;
    else if (c === '>' && !quoted) angle = false;
    else if (!quoted && !angle && (c === ',' || c === ';' || c === '\n')) {
      parts.push(cur);
      cur = '';
      continue;
    }
    cur += c;
  }
  parts.push(cur);
  const out: string[] = [];
  let pending = '';
  for (const p of parts) {
    const t = p.trim();
    if (!t) continue;
    if (!t.includes('@')) {
      pending = pending ? `${pending}, ${t}` : t;
      continue;
    }
    if (pending && /^[^<]*<[^>]*@[^>]*>$/.test(t)) out.push(`${pending}, ${t}`);
    else out.push(...(pending ? [pending, t] : [t]));
    pending = '';
  }
  if (pending) out.push(pending);
  return out;
}

/** "Name <a@b>, \"Doe, John\" <c@d>, e@f" -> Address[] */
export function parseAddresses(s: string): Address[] {
  return splitAddressList(s).map((p) => {
    const m = p.match(/^(.*)<([^>]+)>$/);
    if (!m) return { email: p };
    let name = m[1].trim();
    if (name.startsWith('"') && name.endsWith('"') && name.length >= 2) name = name.slice(1, -1).replace(/\\(.)/g, '$1');
    return { name: name || null, email: m[2].trim() };
  });
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

/** Sync progress without its noun, for when the full text doesn't fit:
 *  "Downloading messages: 31,295 left" → "Downloading: 31,295 left". */
export function shortProgress(detail: string): string {
  return detail.replace(/\s+(messages|headers)\b/gi, '').trim();
}
