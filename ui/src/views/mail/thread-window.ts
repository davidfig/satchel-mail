// A windowed, bounded cache over a thread list of any size.
//
// The list is addressed by absolute index (offset paging), sized by a server
// count, and only a bounded number of pages is kept in memory (LRU). Any refresh
// replaces the cached window wholesale so offsets stay consistent.

import { api } from '../../api';
import type { ThreadQuery, ThreadSummary } from '../../types';

export const PAGE = 100;
const MAX_PAGES = 30;

export class ThreadWindow {
  total = 0;
  loaded = false;
  private pages = new Map<number, ThreadSummary[]>();
  private inflight = new Map<number, Promise<void>>();
  private gen = 0;
  /** Writes under way that take rows out of the list (see `holdReads`). */
  private writes = new Set<Promise<void>>();

  private base: () => ThreadQuery;
  private onChange: () => void;
  private onError: (e: unknown) => void;

  constructor(base: () => ThreadQuery, onChange: () => void, onError: (e: unknown) => void) {
    this.base = base;
    this.onChange = onChange;
    this.onError = onError;
  }

  /** Reads of the list wait for `write`: until it's committed the core still
   *  lists the rows just taken off screen, and they'd come back. */
  holdReads(write: Promise<unknown>) {
    const done: Promise<void> = write.then(() => {}, () => {}).then(() => { this.writes.delete(done); });
    this.writes.add(done);
  }

  private async settled() {
    while (this.writes.size) await Promise.all(this.writes);
  }

  /** Start over (new mailbox or search). */
  async reset() {
    const g = ++this.gen;
    this.pages.clear();
    this.inflight.clear();
    this.total = 0;
    this.loaded = false;
    this.onChange();
    try {
      await this.settled();
      if (g !== this.gen) return;
      const [count, first] = await Promise.all([api.countThreads(this.base()), api.listThreads({ ...this.base(), limit: PAGE, offset: 0 })]);
      if (g !== this.gen) return;
      this.total = count.total;
      this.pages.set(0, first);
      this.loaded = true;
      this.onChange();
    } catch (e) {
      if (g === this.gen) {
        this.loaded = true;
        this.onError(e);
        this.onChange();
      }
    }
  }

  at(i: number): ThreadSummary | undefined {
    const n = Math.floor(i / PAGE);
    const p = this.pages.get(n);
    if (!p) return undefined;
    // Touch for LRU.
    this.pages.delete(n);
    this.pages.set(n, p);
    return p[i % PAGE];
  }

  /** Make sure rows [first, last) are loaded (or loading). */
  ensure(first: number, last: number) {
    if (!this.loaded) return;
    const end = Math.min(last, this.total);
    for (let n = Math.floor(first / PAGE); n * PAGE < end; n++) {
      if (!this.pages.has(n) && !this.inflight.has(n)) void this.fetch(n);
    }
  }

  /** Row i, loading its page if needed. */
  async get(i: number): Promise<ThreadSummary | undefined> {
    // Again if a refresh or removal dropped the page being read.
    for (;;) {
      if (i < 0 || i >= this.total) return undefined;
      const hit = this.at(i);
      if (hit) return hit;
      const n = Math.floor(i / PAGE);
      const g = this.gen;
      await (this.inflight.get(n) ?? this.fetch(n));
      if (g === this.gen) return this.at(i);
    }
  }

  private fetch(n: number): Promise<void> {
    const p = this.load(n);
    this.inflight.set(n, p);
    return p;
  }

  private async load(n: number) {
    const g = this.gen;
    try {
      await this.settled();
      if (g !== this.gen) return;
      const rows = await api.listThreads({ ...this.base(), limit: PAGE, offset: n * PAGE });
      if (g !== this.gen) return;
      this.pages.set(n, rows);
      while (this.pages.size > MAX_PAGES) this.pages.delete(this.pages.keys().next().value!);
      this.onChange();
    } catch (e) {
      if (g === this.gen) this.onError(e);
    } finally {
      this.inflight.delete(n);
    }
  }

  /** Every row in the list, for Select all. Pages outside the cache are read
   *  without caching them, so a long list doesn't evict the visible window.
   *  Null if the list changed (new folder or search) meanwhile. */
  async all(): Promise<ThreadSummary[] | null> {
    const g = this.gen;
    await this.settled();
    if (g !== this.gen) return null;
    const pages = await Promise.all(Array.from({ length: Math.ceil(this.total / PAGE) }, (_, n) =>
      this.pages.get(n) ?? api.listThreads({ ...this.base(), limit: PAGE, offset: n * PAGE })));
    return g === this.gen ? pages.flat() : null;
  }

  indexOf(threadId: string): number {
    for (const [n, rows] of this.pages) {
      const i = rows.findIndex((t) => t.thread_id === threadId);
      if (i >= 0) return n * PAGE + i;
    }
    return -1;
  }

  find(threadId: string): ThreadSummary | undefined {
    for (const rows of this.pages.values()) {
      const t = rows.find((x) => x.thread_id === threadId);
      if (t) return t;
    }
    return undefined;
  }

  /** Drop threads from the cached rows before the server confirms (optimistic
   *  archive/delete). Later rows shift up. A page left short because the rows
   *  that should fill it aren't cached is kept short (its tail shows as
   *  placeholders) rather than dropped, so the visible rows don't blank out;
   *  the refresh that follows the action fills it in. Returns how many were removed. */
  remove(threadIds: Set<string>): number {
    ++this.gen;
    this.inflight.clear();
    const old = [...this.pages.keys()].sort((a, b) => a - b);
    const next = new Map<number, ThreadSummary[]>();
    let removed = 0;
    for (let k = 0; k < old.length; ) {
      // A run of consecutive cached pages, re-chunked after filtering.
      const start = old[k];
      let rows: ThreadSummary[] = [];
      for (; k < old.length && old[k] === start + rows.length / PAGE; k++) rows = rows.concat(this.pages.get(old[k])!);
      const runEnd = start * PAGE + rows.length;
      const kept = rows.filter((t) => !threadIds.has(t.thread_id));
      const shift = removed;
      removed += rows.length - kept.length;
      // Rows before this run shifted it up by `shift`; pages start on page boundaries.
      const absStart = start * PAGE - shift;
      const firstPage = Math.ceil(absStart / PAGE);
      const atEnd = runEnd >= this.total;
      for (let n = firstPage; ; n++) {
        const chunk = kept.slice(n * PAGE - absStart, (n + 1) * PAGE - absStart);
        if (!chunk.length) break;
        next.set(n, chunk);
        if (chunk.length < PAGE && !atEnd) break;
      }
    }
    this.pages = next;
    this.total = Math.max(0, this.total - removed);
    this.onChange();
    return removed;
  }

  /** Re-fetch the count and rows [first, last); everything else is dropped. */
  async refresh(first: number, last: number) {
    const g = ++this.gen;
    this.inflight.clear();
    try {
      await this.settled();
      if (g !== this.gen) return;
      const count = await api.countThreads(this.base());
      const total = count.total;
      const firstPage = Math.floor(Math.max(0, Math.min(first, total - 1)) / PAGE);
      const lastPage = Math.max(firstPage, Math.floor(Math.max(0, Math.min(last, total) - 1) / PAGE));
      const fetched = await Promise.all(
        Array.from({ length: lastPage - firstPage + 1 }, (_, k) => firstPage + k).map(async (n) => [n, await api.listThreads({ ...this.base(), limit: PAGE, offset: n * PAGE })] as const),
      );
      if (g !== this.gen) return;
      this.total = total;
      this.pages = new Map(fetched);
      this.loaded = true;
      this.onChange();
    } catch (e) {
      if (g === this.gen) this.onError(e);
    }
  }
}
