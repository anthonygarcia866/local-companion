// Page tests (e2e/): Glim's real pages in headless Edge (the Chromium that
// WebView2 runs) on localhost, with a stand-in for Tauri (e2e/mock-bridge.js).
// Nothing reaches the desktop: no Glim, no Rust, no input outside the page.
//
//   npx playwright test            (starts Vite on 127.0.0.1:1420 itself)
//
// Screenshots: e2e-results/shots/ (a CI artifact). One line per check in the
// log (the list reporter). A console error or page error fails the check;
// a console warning is logged.

import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.spec.mjs",
  outputDir: "e2e-results/output",
  // One worker: the approval tests wait on real timers (1.5 s, 3 s).
  workers: 1,
  fullyParallel: false,
  timeout: 60_000,
  retries: 0,
  reporter: [["list"]],
  use: {
    channel: "msedge",
    baseURL: "http://127.0.0.1:1420",
    viewport: { width: 900, height: 800 },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "npx vite --host 127.0.0.1 --port 1420 --strictPort",
    url: "http://127.0.0.1:1420/index.html",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
