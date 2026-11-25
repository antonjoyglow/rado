import { spawn, execFileSync } from "node:child_process";
import { openSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const CDP = "http://127.0.0.1:9222/json/version";
const LIMIT_MS = 15 * 60_000;
const up = () => fetch(CDP, { signal: AbortSignal.timeout(2000) }).then((r) => r.ok, () => false);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Starts the app with start-app.cmd unless one already answers on 9222; the returned teardown stops only what it started. */
export default async function startApp() {
  if (await up()) {
    console.log("WebView2 already answers on 9222: using that app and leaving it running.");
    return;
  }
  const log = join(here, "app.log");
  const out = openSync(log, "w");
  const app = spawn("cmd.exe", ["/d", "/c", join(here, "start-app.cmd")], { stdio: ["ignore", out, out], windowsHide: true });
  const stop = () => {
    try {
      // The tree: cmd, tauri, cargo, vite and the app itself.
      execFileSync("taskkill", ["/PID", String(app.pid), "/T", "/F"], { stdio: "ignore" });
    } catch {
      // Already gone.
    }
  };
  const started = Date.now();
  let said = 0;
  console.log(`Starting Rado with tauri dev (log: ${log}). Usually under a minute; a first Rust build takes 5–10 min.`);
  while (!(await up())) {
    const s = Math.round((Date.now() - started) / 1000);
    if (app.exitCode !== null || app.signalCode) throw new Error(`start-app.cmd exited with ${app.exitCode ?? app.signalCode} after ${s} s; see ${log}`);
    if (Date.now() - started > LIMIT_MS) {
      stop();
      throw new Error(`WebView2 didn't answer on 9222 within ${LIMIT_MS / 60_000} min; see ${log}`);
    }
    if (s - said >= 30) console.log(`  still starting… ${(said = s)} s`);
    await sleep(1000);
  }
  console.log(`App up after ${Math.round((Date.now() - started) / 1000)} s.`);
  return stop;
}
