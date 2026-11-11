import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, watchSnapshot, type Snapshot } from "./api";
import { ago, heat, pillCounts } from "./format";
import { h } from "./dom";

const DRAG_THRESHOLD_PX = 4;

export function mountPill(root: HTMLElement): void {
  const reviews = h("span", { class: "count" });
  const bar = h("span", { class: "br" }, "|");
  const mine = h("span", { class: "count" });
  const pill = h("button", { class: "pill", type: "button" }, h("span", { class: "br" }, "{"), reviews, bar, mine, h("span", { class: "br" }, "}"));
  root.append(pill);

  let snapshot: Snapshot | null = null;
  const render = () => {
    if (!snapshot) return;
    document.documentElement.style.setProperty("--pill-scale", String(snapshot.settings.pillScale / 100));
    const counts = pillCounts(snapshot.items);
    const problem = !(snapshot.hasPat || snapshot.hasGithubToken) || (snapshot.error && !snapshot.lastRefreshed);
    reviews.textContent = problem ? "!" : String(counts.reviews);
    reviews.style.color = problem ? "" : heat(counts.reviews);
    mine.textContent = String(counts.mine);
    mine.style.color = heat(counts.mine);
    bar.hidden = mine.hidden = Boolean(problem);
    pill.dataset.polling = String(snapshot.polling);
    pill.title = snapshot.error
      ? snapshot.error
      : `${counts.reviews} to review · ${counts.mine} of your PR rows need work (a rejection, waiting for author, or an open thread) · refreshed ${ago(snapshot.lastRefreshed, new Date())}`;
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
    if (start) void api.toggleList();
    start = null;
  });
}
