import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// A perf build (VITE_PERF=1, docs/plan/gui-perf.md) keeps component names, so renders can be counted by name.
const perf = !!(globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.VITE_PERF;

// Dev server on a fixed local address (docs/plan/gui-frontend.md F1; port changed, see below). Nothing is fetched from the
// network at runtime: the font is bundled from @fontsource.
export default defineConfig({
  plugins: [react()],
  // 47815, a port few tools use; tauri.conf.json's devUrl must match. See gui/DECISIONS.md.
  server: { host: '127.0.0.1', port: 47815, strictPort: true },
  preview: { host: '127.0.0.1', port: 4173, strictPort: true },
  // One local bundle for a desktop app, not a website: React Flow and markdown make it ~650 kB.
  build: { chunkSizeWarningLimit: 900, ...(perf ? { minify: false } : {}) },
  test: { environment: 'node', include: ['src/**/*.test.ts'] },
});
