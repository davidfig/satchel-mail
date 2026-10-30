// The only place the UI touches a host. Desktop uses Tauri IPC; the web build
// (and the future hosted service) uses HTTP + WebSocket. Nothing else in the
// UI may import Tauri APIs.

import type { ApiError, CoreEvent } from './types';

export interface Transport {
  invoke<T>(cmd: string, args?: unknown): Promise<T>;
  /** Calls a desktop-host command (`open_url`, `save_file`, `choose_folder`, `cli_path`), not the engine. The web has none. */
  host<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  subscribe(fn: (e: CoreEvent) => void): () => void;
  /** Links the host caught opening as popups (desktop: the message iframe's). The web has none. */
  onOpenLink(fn: (url: string) => void): void;
  /** Sets the window title (the browser tab on the web). */
  setTitle(title: string): void;
  /** Whether we may show system notifications; `ask` prompts the user if undecided. */
  canNotify(ask: boolean): Promise<boolean>;
  /** Shows a system notification. `onClick` runs only where the platform reports clicks (the web). */
  notify(title: string, body: string, onClick?: () => void): void;
  readonly kind: 'tauri' | 'http';
}

export class RpcError extends Error {
  code: ApiError['code'];
  constructor(e: ApiError) {
    super(e.message);
    this.code = e.code;
  }
}

function isTauri(): boolean {
  return typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== 'undefined';
}

class TauriTransport implements Transport {
  readonly kind = 'tauri' as const;
  private listeners = new Set<(e: CoreEvent) => void>();
  private started = false;

  async invoke<T>(cmd: string, args?: unknown): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    const res = (await invoke('api', { request: { id: 0, cmd, args } })) as { ok?: T; error?: ApiError };
    if (res.error) throw new RpcError(res.error);
    return res.ok as T;
  }

  async host<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }

  subscribe(fn: (e: CoreEvent) => void) {
    this.listeners.add(fn);
    if (!this.started) {
      this.started = true;
      void import('@tauri-apps/api/event').then(({ listen }) =>
        listen<CoreEvent>('satchel://event', (msg) => {
          for (const l of this.listeners) l(msg.payload);
        }),
      );
    }
    return () => this.listeners.delete(fn);
  }

  onOpenLink(fn: (url: string) => void) {
    void import('@tauri-apps/api/event').then(({ listen }) => listen<string>('satchel://open-link', (msg) => fn(msg.payload)));
  }

  setTitle(title: string) {
    // The sidebar re-renders often; skip the IPC round trip when nothing changed.
    if (document.title === title) return;
    document.title = title;
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow().setTitle(title));
  }

  async canNotify(ask: boolean) {
    const { invoke } = await import('@tauri-apps/api/core');
    const granted = await invoke<boolean | null>('plugin:notification|is_permission_granted');
    if (granted !== null || !ask) return granted === true;
    return (await invoke<string>('plugin:notification|request_permission')) === 'granted';
  }

  notify(title: string, body: string) {
    // Desktop notifications don't report clicks; clicking one brings the app forward.
    void import('@tauri-apps/api/core').then(({ invoke }) => invoke('plugin:notification|notify', { options: { title, body } }));
  }
}

class HttpTransport implements Transport {
  readonly kind = 'http' as const;
  private seq = 1;
  private listeners = new Set<(e: CoreEvent) => void>();
  private ws: WebSocket | null = null;
  private retry = 500;
  private taken: Promise<string | null> | undefined;

  private session(): Promise<string | null> {
    this.taken ??= takeSession();
    return this.taken;
  }

  async invoke<T>(cmd: string, args?: unknown): Promise<T> {
    const body = args === undefined ? { id: this.seq++, cmd } : { id: this.seq++, cmd, args };
    // A 401 means this browser has no session: the user has to open the link
    // `satchel serve` printed once.
    const r = await rpcFetch(await this.session(), JSON.stringify(body));
    if (!r.ok) throw new RpcError({ code: r.status === 401 ? 'auth' : 'network', message: await r.text() });
    const res = (await r.json()) as { ok?: T; error?: ApiError };
    if (res.error) throw new RpcError(res.error);
    return res.ok as T;
  }

  host<T>(cmd: string): Promise<T> {
    return Promise.reject(new Error(`${cmd} is only available in the desktop app`));
  }

  onOpenLink() {
    // Browsers open the iframe's popups themselves.
  }

  private connecting = false;

  private connect() {
    if (this.connecting) return;
    this.connecting = true;
    void this.session().then((session) => {
      this.connecting = false;
      this.open(session);
    });
  }

  private open(session: string | null) {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    // Browsers can't set headers on a WebSocket, so the token rides in the
    // subprotocol list; the server answers with plain `satchel`.
    const protocols = session ? ['satchel', `bearer.${session}`] : undefined;
    const ws = new WebSocket(`${proto}://${location.host}/api/events`, protocols);
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 500;
      // We may have missed events while disconnected.
      for (const l of this.listeners) l({ event: 'data_changed' });
    };
    ws.onmessage = (m) => {
      try {
        const e = JSON.parse(m.data as string) as CoreEvent;
        for (const l of this.listeners) l(e);
      } catch {
        /* ignore malformed */
      }
    };
    ws.onclose = () => {
      this.ws = null;
      setTimeout(() => this.connect(), this.retry);
      this.retry = Math.min(this.retry * 2, 10_000);
    };
  }

  subscribe(fn: (e: CoreEvent) => void) {
    this.listeners.add(fn);
    if (!this.ws) this.connect();
    return () => this.listeners.delete(fn);
  }

  setTitle(title: string) {
    document.title = title;
  }

  async canNotify(ask: boolean) {
    if (!('Notification' in window)) return false;
    if (Notification.permission === 'default' && ask) await Notification.requestPermission();
    return Notification.permission === 'granted';
  }

  notify(title: string, body: string, onClick?: () => void) {
    const n = new Notification(title, { body, tag: 'satchel-new-mail' });
    n.onclick = () => {
      window.focus();
      onClick?.();
      n.close();
    };
  }
}

const SESSION_KEY = 'satchel_session';

function rpcFetch(session: string | null, body: string): Promise<Response> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (session) headers.Authorization = `Bearer ${session}`;
  return fetch('/api/rpc', { method: 'POST', headers, credentials: 'omit', body });
}

function storedSession(): string | null {
  try {
    return localStorage.getItem(SESSION_KEY);
  } catch {
    return null;
  }
}

/** The session token, kept in this origin's storage. Unlike a cookie, storage
 *  is per port, so other servers on 127.0.0.1 never see it. In dev the Vite
 *  proxy adds the token instead.
 *
 *  A new token comes from the printed `#session=` link (the fragment never
 *  reaches the server or its logs; `?session=` is still read for old links).
 *  It replaces the stored one only once a call with it succeeds, so a bad
 *  link can't sign you out. Either way it leaves the address bar. */
async function takeSession(): Promise<string | null> {
  // Opening the link in a tab already on this page changes only the fragment,
  // which doesn't reload; reload so the new token is checked and taken.
  window.addEventListener('hashchange', () => {
    if (new URLSearchParams(location.hash.slice(1)).has('session')) location.reload();
  });
  const url = new URL(location.href);
  const hash = new URLSearchParams(url.hash.slice(1));
  const given = hash.get('session') || url.searchParams.get('session');
  const stored = storedSession();
  if (!url.searchParams.has('session') && !hash.has('session')) return stored;
  url.searchParams.delete('session');
  hash.delete('session');
  const rest = hash.toString();
  history.replaceState(null, '', url.pathname + url.search + (rest ? `#${rest}` : ''));
  if (!given || given === stored) return stored;
  let works = false;
  try {
    const r = await rpcFetch(given, JSON.stringify({ id: 0, cmd: 'list_accounts' }));
    works = r.ok;
  } catch {
    /* offline: keep what we had */
  }
  if (!works) return stored;
  try {
    localStorage.setItem(SESSION_KEY, given);
  } catch {
    /* private mode: keep it for this page only */
  }
  return given;
}

export const transport: Transport = isTauri() ? new TauriTransport() : new HttpTransport();
