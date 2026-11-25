import { defineConfig } from "@playwright/test";

// Drives the real app in WebView2 over CDP; input goes through CDP, never the OS mouse or keyboard. app.mjs starts
// the app with start-app.cmd and stops it afterwards, or uses one that already answers on port 9222.
export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.js",
  globalSetup: "./app.mjs",
  workers: 1,
  reporter: "list",
  outputDir: "../../test-results",
});
