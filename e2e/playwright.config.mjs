import { defineConfig } from "@playwright/test";

const port = 1430;

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.js",
  testIgnore: "webview2/**",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 480, height: 640 },
  },
  webServer: {
    command: `npx vite --host 127.0.0.1 --port ${port} --strictPort`,
    cwd: "..",
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: !process.env.CI,
  },
  outputDir: "../test-results",
});
