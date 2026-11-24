import { test, expect, chromium } from "@playwright/test";

test.describe.configure({ mode: "serial" });

let browser;
let list;
const pages = () => browser.contexts().flatMap((c) => c.pages());
const byView = async (view) => {
  for (const p of pages()) if ((await p.evaluate(() => document.body.dataset.view).catch(() => null)) === view) return p;
  return null;
};
const snapshot = () => list.evaluate(() => window.__TAURI_INTERNALS__.invoke("get_snapshot"));
const textScale = () => list.evaluate(() => getComputedStyle(document.body).getPropertyValue("--text-scale").trim());

test.beforeAll(async () => {
  browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
});
test.afterAll(async () => {
  await browser?.close().catch(() => {});
});

test("the counter loads in WebView2 and open_list opens the list", async () => {
  const pill = await byView("pill");
  expect(pill, "counter page").not.toBeNull();
  expect(await pill.evaluate(() => navigator.userAgent)).toContain("Edg/");
  await pill.evaluate(() => window.__TAURI_INTERNALS__.invoke("open_list"));
  await expect.poll(async () => (list = await byView("list")), { timeout: 10_000 }).not.toBeNull();
});

test("without tokens the list opens on Settings", async () => {
  const s = await snapshot();
  expect(s.hasPat || s.hasGithubToken, "this smoke needs an app with no saved token").toBe(false);
  await expect(list.locator("h1")).toHaveText("Settings");
});

test("text size applies and saves", async () => {
  await list.locator("#settings-tab-general").click();
  const h1 = () => list.evaluate(() => parseFloat(getComputedStyle(document.querySelector("h1")).fontSize));
  await list.getByRole("button", { name: "100%" }).click();
  const before = await h1();
  await list.getByRole("button", { name: "130%" }).click();
  await expect.poll(textScale).toBe("1.3");
  expect(await h1()).toBeGreaterThan(before);
  await expect.poll(async () => (await snapshot()).settings.textScale).toBe(130);
  const input = list.getByLabel("Text size in percent");
  await input.fill("150");
  await input.press("Tab");
  await expect.poll(textScale).toBe("1.5");
  await expect(list.getByTitle("Bigger text")).toBeDisabled();
  await list.getByRole("button", { name: "100%" }).click();
  await expect.poll(textScale).toBe("1");
});

test("add a workspace, switch with the chip and back with Ctrl+arrow", async () => {
  await list.locator("#settings-tab-workspaces").click();
  const name = `E2E ${Date.now()} ✓`;
  await list.getByRole("button", { name: "+ Add workspace" }).click();
  const field = list.getByLabel("Workspace name");
  await field.fill(name);
  await field.press("Tab");
  await expect.poll(async () => (await snapshot()).settings.workspaces.some((w) => w.name === name)).toBe(true);

  const s = (await snapshot()).settings;
  const ids = s.workspaces.map((w) => w.id);
  const start = s.activeWorkspace;
  const at = ids.indexOf(start);
  const next = s.workspaces[at > 0 ? at - 1 : at + 1];
  await list.getByTitle("Back to reviews").click();
  await list.locator(".ws-chip").click();
  await list.getByRole("menuitemradio", { name: next.name }).click();
  await expect.poll(async () => (await snapshot()).settings.activeWorkspace).toBe(next.id);
  await expect(list.locator(".ws-chip")).toContainText(next.name);

  await list.locator("h1").click();
  await list.keyboard.press(at > 0 ? "Control+ArrowRight" : "Control+ArrowLeft");
  await expect.poll(async () => (await snapshot()).settings.activeWorkspace).toBe(start);
});

test("the High contrast theme applies", async () => {
  // A switch to a workspace without a token reopens Settings by itself.
  if ((await list.locator("h1").textContent()) !== "Settings") await list.getByTitle("Settings").click();
  await list.locator("#settings-tab-general").click();
  await list.getByRole("button", { name: "High contrast" }).click();
  await expect(list.locator("html")).toHaveAttribute("data-theme", "highContrast");
  await list.getByRole("button", { name: "System", exact: true }).first().click();
});
