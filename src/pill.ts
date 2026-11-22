import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, watchSnapshot, type Snapshot } from "./api";
import { ago } from "./format";
import { checkedText, counterProblem } from "./status";
import { counterElement, paintCounter } from "./counter";
import { activeWorkspace } from "./workspaces";

const DRAG_THRESHOLD_PX = 4;

export function mountPill(root: HTMLElement): void {
  const parts = counterElement("button");
  const { pill } = parts;
  root.append(pill);

  let snapshot: Snapshot | null = null;
  const render = () => {
    if (!snapshot) return;
    const counts = snapshot.counts;
    const now = new Date();
    const problem = counterProblem(snapshot, snapshot.settings.pollMinutes, now);
    paintCounter(parts, counts, snapshot.settings.counter, snapshot.settings.pillScale, problem);
    pill.dataset.polling = String(snapshot.polling);
    const name = activeWorkspace(snapshot.settings)?.name ?? "";
    pill.title = `${name}: ` + (snapshot.warning ? `${snapshot.warning} · ` : "") + (snapshot.error
      ? `${snapshot.error} · ${checkedText(snapshot.lastRefreshed, now)}`
      : `${counts.reviews} to review · ${counts.mine} of your PR rows need work (a rejection, waiting for author, or an open thread) · refreshed ${ago(snapshot.lastRefreshed, now)}`);
  };
  void watchSnapshot((s) => {
    snapshot = s;
    render();
  });
  setInterval(render, 30_000);
  void listen<boolean>("hover", (e) => (pill.dataset.hover = String(e.payload)));

  // A press that moves drags the window; a press that doesn't is a click.
  let start: { x: number; y: number } | null = null;
  pill.addEventListener("mousedown", (e) => {
    if (e.button === 0) start = { x: e.screenX, y: e.screenY };
  });
  pill.addEventListener("mousemove", (e) => {
    if (start && Math.hypot(e.screenX - start.x, e.screenY - start.y) > DRAG_THRESHOLD_PX) {
      start = null;
      void getCurrentWindow().startDragging();
    }
  });
  pill.addEventListener("mouseup", () => {
    if (start) void api.openList();
    start = null;
  });
}
