import { test, expect } from "@playwright/test";
import { open, calls, emit } from "./harness.js";
import { snapshot, item, member, workspace } from "./fixtures.js";

const docOverflow = (page) => page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);

test("3000 rows render and a re-render stays under 2 s", async ({ page }) => {
  const items = Array.from({ length: 3000 }, (_, i) => item({ id: 10_000 + i, title: `PR number ${i}` }));
  await open(page, snapshot({ items }));
  await expect(page.locator(".row")).toHaveCount(3000, { timeout: 20000 });
  const rerender = await page.evaluate(async (s) => {
    const t = performance.now();
    window.__QA.emit("snapshot", s);
    await new Promise((r) => requestAnimationFrame(() => r()));
    return performance.now() - t;
  }, snapshot({ items }));
  expect(rerender).toBeLessThan(2000);
});

test("long and unicode names don't overflow sideways at 360 px", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 500 });
  const long = "ffff".repeat(60);
  const items = [
    item({ title: `Ünïcödé 🦩 タイトル ${"ص".repeat(40)} RTL mixed`, author: "Zoë 🙂 O'Brien-Ångström" }),
    item({ title: long, branch: `feature/${long}`, members: [member({ repo: "r".repeat(80) }), member({ repo: "s" })] }),
  ];
  const ws = [workspace("a", "工作区 🚀 with a really long workspace name that should truncate")];
  await open(page, snapshot({ items, settings: { workspaces: ws, activeWorkspace: "a" } }));
  expect(await docOverflow(page)).toBeLessThanOrEqual(0);
});

test("an aggregate with 25 members wraps inside the window", async ({ page }) => {
  const members = Array.from({ length: 25 }, (_, i) => member({ repo: `repo-${i}`, agentDraft: i % 3 ? null : { host: "github.com" } }));
  await open(page, snapshot({ items: [item({ members, key: "big" })] }));
  expect(await docOverflow(page)).toBeLessThanOrEqual(0);
});

test("in a 300x200 window the header controls stay reachable", async ({ page }) => {
  await page.setViewportSize({ width: 300, height: 200 });
  await open(page, snapshot({ items: [item(), item(), item()] }));
  for (const t of ["Settings", "Close"]) {
    const box = await page.getByTitle(t, { exact: true }).boundingBox();
    expect(box && box.x + box.width <= 300, t).toBeTruthy();
  }
});

test("keyboard only: Tab reaches a row, Enter opens it, arrows and End move between tabs", async ({ page }) => {
  await open(page, snapshot({ items: [item({ key: "kb1", title: "Keyboard row" }), item({ state: "reReview" })] }));
  for (let i = 0; i < 15; i++) {
    await page.keyboard.press("Tab");
    if (await page.evaluate(() => document.activeElement?.textContent?.includes("Keyboard row"))) break;
  }
  await page.keyboard.press("Enter");
  expect((await calls(page, "open_row")).map((c) => c.args.key)).toEqual(["kb1"]);
  await page.locator("#tab-needsReview").focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.locator("#tab-reReview")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("End");
  await expect(page.locator("#tab-hidden")).toHaveAttribute("aria-selected", "true");
});

test("keyboard only: Copy is reachable by Tab and visible when focused", async ({ page }) => {
  await open(page, snapshot({ items: [item({ key: "kb2" })] }));
  let found = false;
  for (let i = 0; i < 20 && !found; i++) {
    await page.keyboard.press("Tab");
    found = await page.evaluate(() => document.activeElement?.textContent === "Copy");
  }
  expect(found).toBe(true);
  await expect(page.locator(".row .actions").first()).toHaveCSS("opacity", "1");
  await page.keyboard.press("Enter");
  expect((await calls(page, "copy_row")).map((c) => c.args.key)).toEqual(["kb2"]);
  expect(await calls(page, "open_row")).toHaveLength(0);
});

test("the High contrast theme sets data-theme", async ({ page }) => {
  await open(page, snapshot({ settings: { theme: "highContrast" } }));
  await expect(page.locator("html")).toHaveAttribute("data-theme", "highContrast");
});

test("an error and a warning show the status line", async ({ page }) => {
  await open(page, snapshot({ settings: { theme: "light" }, error: "GitHub: rate limited, trying again at 14:05", warning: "settings.json was damaged; a copy is in settings.damaged.json" }));
  await expect(page.locator(".status")).toBeVisible();
});

test("no workspaces, or an unknown active one, doesn't crash the list", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const s = snapshot();
  s.settings.workspaces = [];
  s.settings.activeWorkspace = "";
  await open(page, s);
  await page.locator(".ws-chip").click();
  await page.keyboard.press("Control+ArrowRight");
  const s2 = snapshot();
  s2.settings.activeWorkspace = "missing";
  await emit(page, "snapshot", s2);
  await page.getByTitle("Settings").click();
  expect(errors).toEqual([]);
});

test("Esc in Settings returns to the list; Esc in the list hides it", async ({ page }) => {
  await open(page, snapshot());
  await page.getByTitle("Settings").click();
  await page.keyboard.press("Escape");
  expect(await calls(page, "hide_list")).toHaveLength(0);
  await page.keyboard.press("Escape");
  await expect.poll(async () => (await calls(page, "hide_list")).length).toBe(1);
});

test("HTML in titles shows as text", async ({ page }) => {
  await open(page, snapshot({ items: [item({ title: '<img src=x onerror="window.__xss=1"><b>bold</b>', author: "<script>window.__xss=2</script>" })] }));
  await expect(page.locator(".row .title")).toHaveText('<img src=x onerror="window.__xss=1"><b>bold</b>');
  expect(await page.evaluate(() => window.__xss)).toBeUndefined();
});

test("typed poll minutes outside 1–120 are refused", async ({ page }) => {
  await open(page, snapshot());
  await page.getByTitle("Settings").click();
  await page.locator("#settings-tab-general").click();
  const input = page.getByLabel("Minutes between checks");
  for (const v of ["0", "121"]) {
    const before = (await calls(page, "save_settings")).length;
    await input.fill(v);
    await input.press("Tab");
    await page.waitForTimeout(50);
    expect((await calls(page, "save_settings")).length, `typed ${v}`).toBe(before);
  }
});
