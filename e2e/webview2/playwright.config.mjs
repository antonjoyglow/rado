import { defineConfig } from "@playwright/test";

// Drives the real app in WebView2 over CDP. Start it first with e2e\webview2\start-app.cmd and wait
// for the counter to appear; input goes through CDP, never the OS mouse or keyboard.
export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.js",
  workers: 1,
  reporter: "list",
  outputDir: "../../test-results",
});
