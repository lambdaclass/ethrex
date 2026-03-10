import { defineConfig } from '@playwright/test';

// Vite always serves over HTTPS (basicSsl plugin or real certs)
const baseURL = process.env.BASE_URL ?? 'https://localhost:5173';
const isRemote = !!process.env.BASE_URL;

export default defineConfig({
  testDir: './e2e',
  timeout: 120_000,
  expect: { timeout: 30_000 },
  fullyParallel: false, // tests must run sequentially (registration → simple → sponsored → batch → deploy)
  retries: 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL,
    headless: true,
    // We use Chromium exclusively for CDP WebAuthn virtual authenticator support
    browserName: 'chromium',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
    // Allow self-signed certs for remote HTTPS
    ignoreHTTPSErrors: true,
  },
  // Only start local dev server when not testing against a remote URL
  ...(!isRemote && {
    webServer: {
      command: 'npm run dev -- --port 5173',
      port: 5173,
      reuseExistingServer: true,
      timeout: 30_000,
    },
  }),
});
