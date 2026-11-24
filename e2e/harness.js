import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const mock = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "mock.js"), "utf8");

/** Opens the list with `snap` as get_snapshot's answer. */
export async function open(page, snap) {
  await page.addInitScript(mock);
  await page.addInitScript((s) => {
    window.__e2e.responses.get_snapshot = s;
    window.__e2e.responses.get_autostart = false;
    window.__e2e.responses.extension_status = { folder: "extension", state: "notLoaded" };
  }, snap);
  await page.goto("/?view=list");
  await page.waitForFunction(() => window.__e2e.calls.some((c) => c.cmd === "get_snapshot"));
}

export const calls = (page, cmd) => page.evaluate((c) => window.__e2e.callsOf(c), cmd);
/** Waits until every `cmd` call made so far has answered or failed. */
export const settle = (page, cmd) =>
  page.waitForFunction((c) => window.__e2e.settled.filter((s) => s === c).length === window.__e2e.callsOf(c).length, cmd);
export const emit = (page, event, payload) => page.evaluate(([e, p]) => window.__e2e.emit(e, p), [event, payload]);
export const setFail = (page, cmd, error) => page.evaluate(([c, e]) => (window.__e2e.fail[c] = e), [cmd, error]);
export const clearFail = (page, cmd) => page.evaluate((c) => delete window.__e2e.fail[c], cmd);
export const setDelay = (page, cmd, ms) => page.evaluate(([c, m]) => (window.__e2e.delay[c] = m), [cmd, ms]);
