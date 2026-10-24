import { defineConfig } from 'vite';

// In dev, API calls are proxied to `satchel serve` (default port 7781).
const api = process.env.SATCHEL_API ?? 'http://127.0.0.1:7781';

export default defineConfig({
  clearScreen: false,
  server: {
    port: 7780,
    strictPort: true,
    proxy: {
      '/api': {
        target: api,
        ws: true,
        changeOrigin: true,
        headers: process.env.SATCHEL_SESSION_TOKEN ? { Authorization: `Bearer ${process.env.SATCHEL_SESSION_TOKEN}` } : {},
      },
    },
  },
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true },
});
