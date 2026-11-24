import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const mock = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "mock.js"), "utf8");

/** Opens the list (or `view`) with `snap` as get_snapshot's answer. `setup` runs in the page before the app. */
export async function open(page, snap, { view = "list", setup, extensionStatus = { folder: "C:\\x", state: "notLoaded" } } = {}) {
  await page.addInitScript(mock);
  await page.addInitScript(
    ([s, ext]) => {
      window.__QA.responses.get_snapshot = s;
      window.__QA.responses.get_autostart = false;
      window.__QA.responses.extension_status = ext;
    },
    [snap, extensionStatus],
  );
  if (setup) await page.addInitScript(setup);
  await page.goto(`/?view=${view}`);
  await page.waitForFunction(() => window.__QA.calls.some((c) => c.cmd === "get_snapshot"));
}

export const calls = (page, cmd) => page.evaluate((c) => window.__QA.callsOf(c), cmd);
export const emit = (page, event, payload) => page.evaluate(([e, p]) => window.__QA.emit(e, p), [event, payload]);
export const setFail = (page, cmd, error) => page.evaluate(([c, e]) => (window.__QA.fail[c] = e), [cmd, error]);
export const clearFail = (page, cmd) => page.evaluate((c) => delete window.__QA.fail[c], cmd);
export const setDelay = (page, cmd, ms) => page.evaluate(([c, m]) => (window.__QA.delay[c] = m), [cmd, ms]);
