// What Satchel and golemdocs' document viewer say to each other (0062), as
// golemdocs' `client/src/viewer/protocol.ts` defines it, and where the vendored
// build is. No DOM and no imports here, so it can be tested in Node; the frame
// itself is `docviewer.ts`.

// The build in ui/public/golemdocs/<GOLEMDOCS_BUILD>/. scripts/vendor-golemdocs
// writes these two lines; change them by running it.
export const GOLEMDOCS_COMMIT = '1efae91a0b84d7332c84525f1e4a077df969e8c8';
export const GOLEMDOCS_BUILD = '1efae91a0b84';

/** Sent to the viewer once it is ready. `bytes` is transferred. */
export type ToViewer = { type: 'golemdocs:open'; bytes: ArrayBuffer; name: string; dark?: boolean; locale?: string };

/** What the viewer sends back. */
export type FromViewer =
  | { type: 'golemdocs:ready' }
  | { type: 'golemdocs:opened'; name: string; format: string; pages: number }
  | { type: 'golemdocs:error'; message: string }
  | { type: 'golemdocs:link'; url: string }
  | ({ type: 'golemdocs:key' } & HostKey);

/** A key the viewer had no use for, passed on to us as `KeyboardEvent` has it. */
export interface HostKey {
  key: string;
  code: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  repeat: boolean;
}

/** A message from the viewer, or null for anything else. The frame runs
 *  untrusted files, so nothing it sends is taken on trust. */
export function readFromViewer(data: unknown): FromViewer | null {
  if (!data || typeof data !== 'object') return null;
  const m = data as Record<string, unknown>;
  const str = (v: unknown, max = 2000) => (typeof v === 'string' && v.length <= max ? v : null);
  switch (m.type) {
    case 'golemdocs:ready':
      return { type: m.type };
    case 'golemdocs:opened':
      return { type: m.type, name: str(m.name) ?? '', format: str(m.format, 40) ?? '', pages: typeof m.pages === 'number' ? m.pages : 0 };
    case 'golemdocs:error':
      return { type: m.type, message: str(m.message) ?? '' };
    case 'golemdocs:link': {
      const url = str(m.url, 8000);
      return url ? { type: m.type, url } : null;
    }
    case 'golemdocs:key': {
      const key = str(m.key, 32);
      if (!key) return null;
      const flag = (v: unknown) => v === true;
      return { type: m.type, key, code: str(m.code, 32) ?? '', shiftKey: flag(m.shiftKey), ctrlKey: flag(m.ctrlKey), altKey: flag(m.altKey), metaKey: flag(m.metaKey), repeat: flag(m.repeat) };
    }
    default:
      return null;
  }
}

/** The viewer page under `base`, where the host serves `ui/public/golemdocs/`. */
export function viewerUrl(base: string): string {
  return `${base.replace(/\/?$/, '/')}${GOLEMDOCS_BUILD}/viewer.html`;
}
