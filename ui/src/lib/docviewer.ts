// golemdocs' read-only document viewer (0062), in a frame of its own: Word,
// OpenDocument, Markdown, web pages and the rest, drawn from golemdocs' model
// of the file rather than by the webview. The frame is sandboxed without
// `allow-same-origin`, so a parser bug can reach nothing of the app's (its
// DOM, storage or the desktop's IPC), and its policy lets it fetch nothing
// but its own files. Nothing of it loads until a document is shown.

import { h } from './dom';
import { readFromViewer, viewerUrl, type FromViewer, type ToViewer } from './golemdocs';
import { transport } from '../transport';

export interface DocViewerOptions {
  /** The file; copied, so the caller's bytes stay as they are. */
  bytes: Uint8Array;
  /** Its name, whose extension says how it is read. */
  name: string;
  /** The frame's accessible title. */
  title: string;
  onLink(url: string): void;
  onError(message: string): void;
  onOpened?(info: Extract<FromViewer, { type: 'golemdocs:opened' }>): void;
  /** A key pressed in the document that the viewer had no use for (Escape,
   *  ←/→ when the pages fit, letters), as a keydown event dispatched nowhere. */
  onKey?(e: KeyboardEvent): void;
}

/** How long the viewer has to say it is ready before we give up on it. */
const START_TIMEOUT = 20_000;

/** Whether the app is showing dark colors, so the document is drawn dark too. */
function dark(): boolean {
  const theme = document.documentElement.dataset.theme;
  return theme ? theme === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
}

/** A frame showing one document. `destroy` removes it, which ends its
 *  worker and frees its GPU memory with it. */
export function docViewer(opts: DocViewerOptions): { el: HTMLIFrameElement; destroy(): void } {
  const frame = h('iframe.viewer-frame.doc-frame', {
    src: viewerUrl(transport.docViewerBase()),
    sandbox: 'allow-scripts',
    referrerpolicy: 'no-referrer',
    title: opts.title,
  }) as HTMLIFrameElement;
  let sent = false;
  let done = false;
  // A page out of sight is throttled hard (WebKit in a hidden window), so
  // the frame may take far longer to load: wait until it can be seen.
  const startTimer = (): number => window.setTimeout(() => {
    if (document.hidden) timer = startTimer();
    else fail('The document viewer didn’t start.');
  }, START_TIMEOUT);
  let timer = startTimer();
  function fail(message: string) {
    window.clearTimeout(timer);
    if (!done) opts.onError(message);
  }
  // Its messages come with origin "null": the only way to know them is the
  // window they come from.
  const onMessage = (e: MessageEvent) => {
    if (done || !frame.contentWindow || e.source !== frame.contentWindow) return;
    const m = readFromViewer(e.data);
    if (m?.type === 'golemdocs:ready' && !sent) {
      sent = true;
      window.clearTimeout(timer);
      const bytes = opts.bytes.slice().buffer;
      const open: ToViewer = { type: 'golemdocs:open', bytes, name: opts.name, dark: dark(), locale: navigator.language };
      frame.contentWindow.postMessage(open, '*', [bytes]);
    } else if (m?.type === 'golemdocs:opened') opts.onOpened?.(m);
    else if (m?.type === 'golemdocs:error') fail(m.message);
    else if (m?.type === 'golemdocs:link') opts.onLink(m.url);
    // Only while the frame has the focus, so a press is the user's, not the file's.
    else if (m?.type === 'golemdocs:key' && document.activeElement === frame && document.hasFocus()) {
      const { type: _, ...init } = m;
      opts.onKey?.(new KeyboardEvent('keydown', { ...init, cancelable: true }));
    }
  };
  window.addEventListener('message', onMessage);
  return {
    el: frame,
    destroy() {
      done = true;
      window.clearTimeout(timer);
      window.removeEventListener('message', onMessage);
      frame.remove();
    },
  };
}
