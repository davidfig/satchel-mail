// The only place the UI touches a host. Desktop uses Tauri IPC; the web build
// (and the future hosted service) uses HTTP + WebSocket. Nothing else in the
// UI may import Tauri APIs.

import type { ApiError, CoreEvent } from './types';

export interface Transport {
  invoke<T>(cmd: string, args?: unknown): Promise<T>;
  subscribe(fn: (e: CoreEvent) => void): () => void;
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

  async invoke<T>(cmd: string, args?: unknown): Promise<T> {
    const body = args === undefined ? { id: this.seq++, cmd } : { id: this.seq++, cmd, args };
    const post = () =>
      fetch('/api/rpc', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify(body),
      });
    let r = await post();
    if (r.status === 401) {
      // The server restarted with a new session; pick up the fresh cookie and retry once.
      await fetch('/api/session', { credentials: 'same-origin', cache: 'no-store' });
      r = await post();
    }
    if (!r.ok) throw new RpcError({ code: r.status === 401 ? 'auth' : 'network', message: await r.text() });
    const res = (await r.json()) as { ok?: T; error?: ApiError };
    if (res.error) throw new RpcError(res.error);
    return res.ok as T;
  }

  private connect() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/api/events`);
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
      void fetch('/api/session', { credentials: 'same-origin', cache: 'no-store' }).catch(() => {});
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

export const transport: Transport = isTauri() ? new TauriTransport() : new HttpTransport();
