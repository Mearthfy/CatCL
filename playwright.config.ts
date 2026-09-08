import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './e2e',
  use: { channel: 'msedge', headless: true, baseURL: 'http://127.0.0.1:1420', viewport: { width: 1200, height: 820 } },
  webServer: { command: 'npm run dev', url: 'http://127.0.0.1:1420', reuseExistingServer: !process.env.CI },
})
