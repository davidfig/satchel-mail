// Taking conversations out of the list: what's on screen while the core is
// still writing, and which conversation the reader moves on to. Runs the real
// `act` and `ThreadWindow` against a fake server. Run with `npm test`.

import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { beforeEach, describe, test } from 'node:test';
import { parseHTML } from 'linkedom';
import type { ThreadSummary } from '../src/types/index.ts';

// The app's imports omit the `.ts` extension (Vite resolves them); do the same here.
registerHooks({
  resolve(spec, ctx, next) {
    try {
      return next(spec, ctx);
    } catch (e) {
      if (spec.startsWith('.') && !spec.endsWith('.ts')) return next(`${spec}.ts`, ctx);
      throw e;
    }
  },
});

const dom = parseHTML('<!doctype html><html><body></body></html>');
Object.assign(globalThis, { window: globalThis, document: dom.document, Node: dom.Node, Element: dom.Element, location: { origin: 'http://127.0.0.1', hash: '', search: '' } });
const { api } = await import('../src/api.ts');
const { store } = await import('../src/state.ts');
const { act } = await import('../src/views/mail/actions.ts');
const actions = await import('../src/views/mail/actions.ts');
const { ThreadWindow } = await import('../src/views/mail/thread-window.ts');

const row = (id: string) => ({ thread_id: id, mailbox_ids: ['spam'], unread_count: 0, participants: [] }) as unknown as ThreadSummary;

/** The core: what it lists, and writes that wait until a test lets them through. */
let server: string[];
let writes: { commit: () => void; fail: () => void }[];
/** Lets the writes (and the reads waiting on them) go ahead. */
const settle = () => new Promise((r) => setTimeout(r, 5));
/** Lets the nth write commit, once it has reached the core. */
const commit = async (n = 0) => {
  await settle();
  writes[n].commit();
};

function fakeServer(ids: string[]) {
  server = ids;
  writes = [];
  const a = api as unknown as Record<string, unknown>;
  a.countThreads = async () => ({ total: server.length });
  a.listThreads = async (q: { limit: number; offset: number }) => server.slice(q.offset, q.offset + q.limit).map(row);
  a.getThread = async (id: string) => ({ thread_id: id, messages: [{ id }] });
  const write = (ids: string[]) => new Promise<void>((resolve, reject) => {
    writes.push({
      commit: () => {
        server = server.filter((t) => !ids.includes(t));
        resolve();
      },
      fail: () => reject(new Error('nope')),
    });
  });
  a.trash = write;
  a.archive = write;
  a.markSpam = async () => ({ senders: [], also_moved: 0, invites_removed: 0 });
}

/** The parts of MailView that `act` drives. */
async function viewOf(open: string | null, cursor = -1) {
  const view = {
    list: new ThreadWindow(() => ({}) as never, () => {}, () => {}),
    cursor,
    selected: new Map<string, ThreadSummary>(),
    openThread: open ? { thread_id: open, messages: [{ id: open }] } : null,
    openSummary: null,
    readerThread: open,
    closed: 0,
    renderHeader() {},
    renderRows() {},
    renderEmptyReader() {
      view.closed++;
      view.openThread = null;
      view.readerThread = null;
    },
    showThread(id: string) {
      store.set({ selectedThread: id });
      view.readerThread = id;
      view.openThread = { thread_id: id, messages: [{ id }] };
      return Promise.resolve();
    },
    messageIds: (ts: ThreadSummary[]) => actions.messageIds(view as never, ts),
    targetThreads: () => actions.targetThreads(view as never),
    refresh: () => view.list.refresh(0, 100),
  };
  store.set({ selectedThread: open });
  await view.list.reset();
  return view;
}

const listed = (view: Awaited<ReturnType<typeof viewOf>>) => Array.from({ length: view.list.total }, (_, i) => view.list.at(i)?.thread_id);

beforeEach(() => {
  store.state.source = { kind: 'role', role: 'junk', label: 'Spam' };
  store.state.mailboxes = [];
});

describe('a removed conversation stays off the list', () => {
  test('through a refresh while the core is still writing', async () => {
    fakeServer(['a', 'b', 'c']);
    const view = await viewOf(null, 0);
    const done = act(view as never, 'trash', [view.list.at(0)!]);
    assert.deepEqual(listed(view), ['b', 'c'], 'gone at once');

    // A sync tick lands before the delete is written; the core would still list it.
    void view.list.refresh(0, 100);
    await settle();
    assert.deepEqual(listed(view), ['b', 'c'], 'a read of the old rows put it back');

    await commit();
    await done;
    await settle();
    assert.deepEqual(listed(view), ['b', 'c']);
    assert.equal(view.list.total, 2);
  });

  test('through a refresh already on its way when the delete starts', async () => {
    fakeServer(['a', 'b']);
    const view = await viewOf(null, 0);
    const refreshing = view.list.refresh(0, 100);
    const done = act(view as never, 'trash', [view.list.at(0)!]);
    await refreshing;
    assert.deepEqual(listed(view), ['b']);
    await commit();
    await done;
  });

  test('through a page loaded while the core is still writing', async () => {
    fakeServer(Array.from({ length: 150 }, (_, i) => `t${i}`));
    const view = await viewOf(null, 0);
    const done = act(view as never, 'trash', [view.list.at(0)!]);
    await settle();
    view.list.ensure(100, 149);
    await settle();
    assert.equal(view.list.at(100), undefined, 'page 1 read from before the delete is a row off');
    await commit();
    await done;
    await settle();
    view.list.ensure(100, 149);
    await settle();
    assert.equal(view.list.at(100)?.thread_id, 't101');
  });

  test('through several deletes in a row', async () => {
    fakeServer(['a', 'b', 'c', 'd']);
    const view = await viewOf(null, 0);
    const first = act(view as never, 'trash', [view.list.at(0)!]);
    const second = act(view as never, 'trash', [view.list.at(0)!]);
    await settle();
    void view.list.refresh(0, 100);
    await commit();
    await first;
    await settle();
    assert.deepEqual(listed(view), ['c', 'd'], 'the second delete is still being written');
    await commit(1);
    await second;
    await settle();
    assert.deepEqual(listed(view), ['c', 'd']);
  });

  test('and comes back if the delete failed', async () => {
    fakeServer(['a', 'b']);
    const view = await viewOf(null, 0);
    const done = act(view as never, 'trash', [view.list.at(0)!]);
    await settle();
    assert.deepEqual(listed(view), ['b']);
    writes[0].fail();
    await done;
    await settle();
    assert.deepEqual(listed(view), ['a', 'b']);
  });
});

describe('the reader moves on to the next conversation', () => {
  test('after deleting from Spam', async () => {
    fakeServer(['a', 'b', 'c']);
    const view = await viewOf('a', 0);
    const done = act(view as never, 'trash');
    assert.equal(view.readerThread, 'b');
    assert.equal(store.state.selectedThread, 'b');
    assert.equal(view.cursor, 0);
    assert.equal(view.closed, 0, 'no empty pane in between');
    await commit();
    await done;
  });

  test('after Spam: delete and block', async () => {
    fakeServer(['a', 'b']);
    const view = await viewOf('a', 0);
    const done = act(view as never, 'block');
    assert.equal(view.readerThread, 'b');
    await commit();
    await done;
  });

  test('deleting again acts on the next one, not the one just deleted', async () => {
    fakeServer(['a', 'b', 'c']);
    const view = await viewOf('a', 0);
    const first = act(view as never, 'trash');
    const second = act(view as never, 'trash');
    assert.equal(view.readerThread, 'c');
    await settle();
    assert.equal(writes.length, 2);
    await commit();
    await commit(1);
    await Promise.all([first, second]);
    assert.deepEqual(server, ['c']);
  });

  test('from the row below the deleted one, even with others removed above it', async () => {
    fakeServer(['a', 'b', 'c', 'd', 'e']);
    const view = await viewOf('c', 2);
    const done = act(view as never, 'trash', [view.list.at(0)!, view.list.at(2)!]);
    assert.equal(view.readerThread, 'd');
    assert.equal(view.cursor, 1);
    await commit();
    await done;
  });

  test('back to the row above when the last one goes', async () => {
    fakeServer(['a', 'b']);
    const view = await viewOf('b', 1);
    const done = act(view as never, 'trash');
    assert.equal(view.readerThread, 'a');
    assert.equal(view.cursor, 0);
    await commit();
    await done;
  });

  test('the reader closes when nothing is left', async () => {
    fakeServer(['a']);
    const view = await viewOf('a', 0);
    const done = act(view as never, 'trash');
    assert.equal(view.readerThread, null);
    assert.equal(store.state.selectedThread, null);
    assert.equal(view.closed, 1);
    await commit();
    await done;
  });

  test('a row not loaded yet opens once it is', async () => {
    fakeServer(Array.from({ length: 150 }, (_, i) => `t${i}`));
    const view = await viewOf('t99', 99);
    const done = act(view as never, 'trash');
    assert.equal(view.closed, 1, 'nothing to show yet');
    await commit();
    await done;
    await settle();
    assert.equal(view.readerThread, 't100');
  });

  test('a row elsewhere leaves the reader alone', async () => {
    fakeServer(['a', 'b', 'c']);
    const view = await viewOf('a', 2);
    const done = act(view as never, 'trash', [view.list.at(2)!]);
    assert.equal(view.readerThread, 'a');
    assert.equal(view.closed, 0);
    await commit();
    await done;
  });

  test('in Drafts the reader closes, since opening a draft edits it', async () => {
    store.state.source = { kind: 'role', role: 'drafts', label: 'Drafts' };
    fakeServer(['a', 'b']);
    const view = await viewOf('a', 0);
    const done = act(view as never, 'trash');
    assert.equal(view.readerThread, null);
    await commit();
    await done;
  });
});
