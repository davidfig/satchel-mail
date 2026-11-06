import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { defineConfig, type Plugin } from 'vite';

// In dev, API calls are proxied to `satchel serve` (default port 7781).
const api = process.env.SATCHEL_API ?? 'http://127.0.0.1:7781';
const token = process.env.SATCHEL_SESSION_TOKEN;
// The page's own origin; `scripts/run dev` sets the server's allowed origin to the same.
const devOrigin = process.env.SATCHEL_DEV_ORIGIN ?? 'http://127.0.0.1:7780';

// The proxy holds the session token, so it only lends it to requests from our
// own page: a browser sends `Origin` on POSTs and WebSockets, and
// `Sec-Fetch-Site` on everything else. Anything else reaches the API unsigned
// and gets a 401. (A local program can forge these headers; this stops other
// pages, not other processes.)
type ProxyReq = { removeHeader(name: string): void; setHeader(name: string, value: string): void };
type IncomingReq = { headers: Record<string, string | string[] | undefined> };

function authorize(proxyReq: ProxyReq, req: IncomingReq) {
  proxyReq.removeHeader('authorization');
  const origin = req.headers.origin;
  const site = req.headers['sec-fetch-site'];
  // Our page is never framed, so a framed request is someone else's page.
  const dest = req.headers['sec-fetch-dest'];
  const framed = dest === 'iframe' || dest === 'frame';
  const fromPage = origin !== undefined ? origin === devOrigin : site === 'same-origin';
  if (token && fromPage && !framed) proxyReq.setHeader('Authorization', `Bearer ${token}`);
}

// No other page may frame ours (clickjacking); the same as `satchel serve`.
const noFraming = { 'Content-Security-Policy': "frame-ancestors 'none'", 'X-Frame-Options': 'DENY' };

const TYPES: Record<string, string> = { html: 'text/html', js: 'text/javascript', css: 'text/css', json: 'application/json', webp: 'image/webp' };

// The document viewer (0062) with the headers `satchel serve` and the desktop
// app give it (satchel_core::doc_viewer): framed by our page only, its files
// readable from its sandbox, its page allowed nothing but its own origin.
function docViewer(): Plugin {
  const root = path.resolve(import.meta.dirname, 'public/golemdocs');
  return {
    name: 'satchel-doc-viewer',
    configureServer(server) {
      server.middlewares.use('/golemdocs', (req, res, next) => {
        const file = decodeURIComponent((req.url ?? '').split('?')[0]).replace(/^\//, '');
        if (!/^[\w.-]+(\/[\w-][\w.-]*)*$/.test(file) || file.includes('..')) return next();
        readFile(path.join(root, file)).then((body) => {
          res.setHeader('Content-Type', TYPES[path.extname(file).slice(1)] ?? 'text/plain');
          res.setHeader('X-Content-Type-Options', 'nosniff');
          if (file.endsWith('viewer.html')) {
            const o = devOrigin;
            res.setHeader('Content-Security-Policy', `default-src 'none'; script-src ${o}; worker-src ${o} data:; connect-src ${o}; style-src ${o}; img-src ${o} blob: data:; font-src ${o}; base-uri 'none'; form-action 'none'; frame-ancestors ${o}`);
            res.setHeader('X-Frame-Options', 'SAMEORIGIN');
          } else res.setHeader('Access-Control-Allow-Origin', '*');
          res.end(body);
        }, () => next());
      });
    },
  };
}

export default defineConfig({
  clearScreen: false,
  plugins: [docViewer()],
  server: {
    host: '127.0.0.1',
    port: 7780,
    strictPort: true,
    headers: noFraming,
    proxy: {
      '/api': {
        target: api,
        ws: true,
        changeOrigin: true,
        configure: (proxy) => {
          proxy.on('proxyReq', authorize);
          proxy.on('proxyReqWs', authorize);
        },
      },
    },
  },
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true },
});
