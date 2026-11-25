// Playwright setup of the end-to-end tests: the `*.spec.ts` files of `e2e/` run one after the other against the real
// hub in the system's Chrome, prepared once by `e2e/global.ts`.

import { defineConfig } from '@playwright/test';

// The system's Chrome is used (`channel`), so nothing is downloaded. One worker: the tests share one hub and
// pair it one after the other.
export default defineConfig({
  testDir: './e2e',
  testMatch: /.*\.spec\.ts/,
  globalSetup: './e2e/global.ts',
  workers: 1,
  fullyParallel: false,
  timeout: 30_000,
  reporter: 'list',
  use: {
    channel: 'chrome',
    baseURL: 'http://127.0.0.1:17697',
    trace: 'retain-on-failure',
    screenshot: 'off',
    video: 'off',
  },
});
