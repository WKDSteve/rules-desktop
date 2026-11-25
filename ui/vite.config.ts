// The Vite and Vitest setup of the page: Svelte plugin, the build into `dist` that the hub embeds, and the unit tests
// under `src/` and `e2e/`.

import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

// The page is served by the hub on a fixed origin. Nothing is inlined: no
// data: URL, no inline script (the CSP of the shell allows 'self' only).
export default defineConfig({
  plugins: [svelte()],
  base: './',
  build: {
    outDir: 'dist',
    target: 'es2022',
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    sourcemap: false,
  },
  test: {
    environment: 'node',
    // e2e/*.test.ts: the cleanup of the browser tests, checked without a hub (the browser tests are e2e/*.spec.ts).
    include: ['src/**/*.test.ts', 'e2e/**/*.test.ts'],
  },
});
