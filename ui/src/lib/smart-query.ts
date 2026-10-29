// The smart folder form and its search query, kept in step: the form's fields
// write the query, and a typed query fills in the fields it can. Anything
// the form has no field for (a negated term, `before:`, `with:`) is kept as is.

/** Choices for "Received", as `newer_than:` values. */
export const AGES: [string, string][] = [['', 'Any time'], ['1d', 'Today'], ['7d', 'In the last week'], ['30d', 'In the last month'], ['365d', 'In the last year']];

export interface QueryFields {
  from: string;
  to: string;
  subject: string;
  words: string;
  unread: boolean;
  flagged: boolean;
  attachment: boolean;
  /** A `newer_than:` value from AGES, or ''. */
  age: string;
  /** Account addresses to search (`account:`); empty means all. */
  accounts: string[];
  /** `in:anywhere`: include Trash and Junk. */
  anywhere: boolean;
  /** Terms the form has no field for, kept verbatim. */
  other: string[];
}

export function emptyFields(): QueryFields {
  return { from: '', to: '', subject: '', words: '', unread: false, flagged: false, attachment: false, age: '', accounts: [], anywhere: false, other: [] };
}

/** Split respecting double quotes (like the core's parser). */
export function tokens(s: string): string[] {
  const out: string[] = [];
  let cur = '';
  let q = false;
  for (const c of s) {
    if (c === '"') q = !q;
    if (/\s/.test(c) && !q) {
      if (cur) out.push(cur);
      cur = '';
    } else cur += c;
  }
  if (cur) out.push(cur);
  return out;
}

const unquote = (v: string) => v.replace(/^"|"$/g, '');
const quote = (v: string) => (/\s/.test(v) ? `"${v.replace(/"/g, '')}"` : v);
const list = (v: string) => v.split(',').map((x) => x.trim()).filter(Boolean);

/** Read a query into form fields. `accounts` are the addresses that exist. */
export function parseFields(query: string, accounts: string[] = []): QueryFields {
  const f = emptyFields();
  const words: string[] = [];
  const known = new Set(accounts.map((a) => a.toLowerCase()));
  for (const t of tokens(query)) {
    const i = t.indexOf(':');
    const k = i > 0 ? t.slice(0, i).toLowerCase() : '';
    const v = i > 0 ? unquote(t.slice(i + 1)) : '';
    const lv = v.toLowerCase();
    if (t.startsWith('-') || !v) {
      if (!t.startsWith('-') && i < 0) words.push(t);
      else f.other.push(t);
    } else if (k === 'from' && !f.from) f.from = list(v).join(', ');
    else if (k === 'to' && !f.to) f.to = list(v).join(', ');
    else if (k === 'subject' && !f.subject) f.subject = v;
    else if (k === 'is' && lv === 'unread') f.unread = true;
    else if (k === 'is' && (lv === 'flagged' || lv === 'starred')) f.flagged = true;
    else if (k === 'has' && lv === 'attachment') f.attachment = true;
    else if (k === 'newer_than' && !f.age && AGES.some(([a]) => a === lv)) f.age = lv;
    else if (k === 'in' && lv === 'anywhere') f.anywhere = true;
    else if (k === 'account' && !f.accounts.length && list(lv).every((a) => known.has(a))) f.accounts = list(lv);
    else f.other.push(t);
  }
  f.words = words.join(' ');
  return f;
}

/** Write form fields as a query. */
export function formatFields(f: QueryFields): string {
  const parts: string[] = [];
  const addrs = (op: string, v: string) => {
    const l = list(v).map((x) => x.replace(/\s+/g, ''));
    if (l.length) parts.push(`${op}:${l.join(',')}`);
  };
  addrs('from', f.from);
  addrs('to', f.to);
  if (f.subject.trim()) parts.push(`subject:${quote(f.subject.trim())}`);
  if (f.unread) parts.push('is:unread');
  if (f.flagged) parts.push('is:flagged');
  if (f.attachment) parts.push('has:attachment');
  if (f.age) parts.push(`newer_than:${f.age}`);
  if (f.accounts.length) parts.push(`account:${f.accounts.join(',')}`);
  if (f.anywhere) parts.push('in:anywhere');
  parts.push(...f.other);
  if (f.words.trim()) parts.push(f.words.trim());
  return parts.join(' ');
}

/** Starting points offered when making a smart folder. */
export const TEMPLATES: { name: string; query: string }[] = [
  { name: 'Unread', query: 'is:unread' },
  { name: 'Flagged', query: 'is:flagged' },
  { name: 'With attachments', query: 'has:attachment' },
  { name: 'This week', query: 'newer_than:7d' },
];
