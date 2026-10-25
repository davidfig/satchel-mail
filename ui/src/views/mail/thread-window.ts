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

  constructor(
    private base: () => ThreadQuery,
    private onChange: () => void,
    private onError: (e: unknown) => void,
  ) {}

  /** Start over (new mailbox or search). */
  async reset() {
    const g = ++this.gen;
    this.pages.clear();
    this.inflight.clear();
    this.total = 0;
    this.loaded = false;
    this.onChange();
    try {
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
    if (i < 0 || i >= this.total) return undefined;
    const hit = this.at(i);
    if (hit) return hit;
    const n = Math.floor(i / PAGE);
    await (this.inflight.get(n) ?? this.fetch(n));
    return this.at(i);
  }

  private fetch(n: number): Promise<void> {
    const p = this.load(n);
    this.inflight.set(n, p);
    return p;
  }

  private async load(n: number) {
    const g = this.gen;
    try {
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

  /** Re-fetch the count and rows [first, last); everything else is dropped. */
  async refresh(first: number, last: number) {
    const g = ++this.gen;
    this.inflight.clear();
    try {
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
