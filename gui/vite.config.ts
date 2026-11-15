import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Dev server on a fixed local address (docs/plan/gui-frontend.md F1; port changed, see below). Nothing is fetched from the
// network at runtime: the font is bundled from @fontsource.
export default defineConfig({
  plugins: [react()],
  // 5174, not 5173: another app on this machine (a container) holds 5173. See gui/DECISIONS.md.
  server: { host: '127.0.0.1', port: 5174, strictPort: true },
  preview: { host: '127.0.0.1', port: 4173, strictPort: true },
  test: { environment: 'node', include: ['src/**/*.test.ts'] },
});
