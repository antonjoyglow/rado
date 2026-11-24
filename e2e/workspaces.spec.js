import { test, expect } from "@playwright/test";
import { open, calls, emit, setDelay, setFail, settle } from "./harness.js";
import { snapshot, item, workspace } from "./fixtures.js";

const three = () => [workspace("a", "Alpha"), workspace("b", "Beta"), workspace("c", "Gamma")];

test.describe("switching workspaces", () => {
  test("Ctrl+arrows step through workspaces without wrapping at the start", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "a" } }));
    await page.keyboard.press("Control+ArrowLeft");
    await page.keyboard.press("Control+ArrowRight");
    expect((await calls(page, "switch_workspace")).map((c) => c.args.id)).toEqual(["b"]);
  });

  test("Ctrl+→ at the last workspace does nothing", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "c" } }));
    await page.keyboard.press("Control+ArrowRight");
    await page.keyboard.press("Control+ArrowLeft");
    expect((await calls(page, "switch_workspace")).map((c) => c.args.id)).toEqual(["b"]);
  });

  test("Ctrl+→ in a text field moves by word and doesn't switch", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "a" } }));
    await page.getByTitle("Settings").click();
    await page.getByLabel("My teams").focus();
    await page.keyboard.press("Control+ArrowRight");
    expect(await calls(page, "switch_workspace")).toHaveLength(0);
  });

  test("Ctrl+Shift+→ and Ctrl+Alt+→ don't switch", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "a" } }));
    await page.keyboard.press("Control+Shift+ArrowRight");
    await page.keyboard.press("Control+Alt+ArrowRight");
    expect(await calls(page, "switch_workspace")).toHaveLength(0);
  });

  test("the chip's picker works by keyboard and Esc returns focus to the chip", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "a" } }));
    const chip = page.locator(".ws-chip");
    await expect(chip).toContainText("Alpha");
    await chip.click();
    await expect(page.locator(".ws-picker")).toBeVisible();
    await expect(page.getByRole("menuitemradio", { name: "Alpha" })).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    expect((await calls(page, "switch_workspace")).map((c) => c.args.id)).toEqual(["b"]);
    await expect(page.locator(".ws-picker")).toBeHidden();
    await chip.click();
    await page.keyboard.press("Escape");
    await expect(page.locator(".ws-picker")).toBeHidden();
    await expect(chip).toBeFocused();
    expect(await calls(page, "hide_list")).toHaveLength(0);
  });

  test("picking the active workspace doesn't switch; Manage opens Settings", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "a" } }));
    await page.locator(".ws-chip").click();
    await page.getByRole("menuitemradio", { name: "Alpha" }).click();
    expect(await calls(page, "switch_workspace")).toHaveLength(0);
    await page.locator(".ws-chip").click();
    await page.getByRole("menuitem", { name: "Manage workspaces…" }).click();
    await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  });

  test("a snapshot after a switch updates the chip and the rows", async ({ page }) => {
    await open(page, snapshot({ settings: { workspaces: three(), activeWorkspace: "a" }, items: [item({ title: "Alpha PR" })] }));
    await emit(page, "snapshot", snapshot({ settings: { workspaces: three(), activeWorkspace: "b" }, items: [item({ title: "Beta PR" })] }));
    await expect(page.locator(".ws-chip")).toContainText("Beta");
    await expect(page.locator(".row .title")).toHaveText("Beta PR");
  });

  test("rapid Ctrl+→ presses with a slow backend all ask for the next workspace", async ({ page }) => {
    const ws = Array.from({ length: 6 }, (_, i) => workspace(`w${i}`, `W${i}`));
    await open(page, snapshot({ settings: { workspaces: ws, activeWorkspace: "w0" } }));
    await setDelay(page, "switch_workspace", 200);
    for (let i = 0; i < 10; i++) await page.keyboard.press("Control+ArrowRight");
    await settle(page, "switch_workspace");
    // The mock sends no new snapshot, so the active workspace stays w0.
    expect(new Set((await calls(page, "switch_workspace")).map((c) => c.args.id))).toEqual(new Set(["w1"]));
  });
});

test.describe("ticket chip", () => {
  test("a Jira chip is titled with the tracker and opens the ticket, not the row", async ({ page }) => {
    const ws = [workspace("a", "A", { tracker: { kind: "jira", site: "example.atlassian.net", keys: ["ABC"] } })];
    await open(page, snapshot({ settings: { workspaces: ws, activeWorkspace: "a" }, items: [item({ key: "row1", ticket: "ABC-342" })] }));
    const chip = page.getByRole("button", { name: "ABC-342" });
    await expect(chip).toHaveAttribute("title", "Open ABC-342 in Jira");
    await chip.click();
    expect((await calls(page, "open_ticket")).map((c) => c.args)).toEqual([{ key: "row1" }]);
    expect(await calls(page, "open_row")).toHaveLength(0);
  });

  test("a Linear chip names Linear", async ({ page }) => {
    const ws = [workspace("a", "A", { tracker: { kind: "linear", workspace: "example", keys: [] } })];
    await open(page, snapshot({ settings: { workspaces: ws, activeWorkspace: "a" }, items: [item({ ticket: "ENG-7" })] }));
    await expect(page.getByRole("button", { name: "ENG-7" })).toHaveAttribute("title", "Open ENG-7 in Linear");
  });

  test("a failed ticket open goes to the status line", async ({ page }) => {
    const ws = [workspace("a", "A", { tracker: { kind: "linear", workspace: "example", keys: [] } })];
    await open(page, snapshot({ settings: { workspaces: ws, activeWorkspace: "a" }, items: [item({ ticket: "ENG-7" })] }));
    await setFail(page, "open_ticket", "No tracker for this workspace");
    await page.getByRole("button", { name: "ENG-7" }).click();
    await expect(page.locator(".status .failed")).toHaveText("No tracker for this workspace");
  });
});

test.describe("text size", () => {
  const general = async (page, snap) => {
    await open(page, snap);
    await page.getByTitle("Settings").click();
    await page.locator("#settings-tab-general").click();
  };

  test("chips, −/+ in steps of 5 and typed values; the limits disable the buttons", async ({ page }) => {
    await general(page, snapshot());
    const input = page.getByLabel("Text size in percent");
    await page.getByRole("button", { name: "130%" }).click();
    await expect(input).toHaveValue("130");
    await page.getByTitle("Bigger text").click();
    await expect(input).toHaveValue("135");
    await input.fill("150");
    await input.press("Tab");
    await expect(page.getByTitle("Bigger text")).toBeDisabled();
    await input.fill("80");
    await input.press("Tab");
    await expect(page.getByTitle("Smaller text")).toBeDisabled();
    expect((await calls(page, "save_settings")).map((c) => c.args.settings.textScale).at(-1)).toBe(80);
  });

  test("out-of-range typed values are refused", async ({ page }) => {
    await general(page, snapshot());
    const input = page.getByLabel("Text size in percent");
    for (const v of ["79", "151", "-100"]) {
      const before = (await calls(page, "save_settings")).length;
      await input.fill(v);
      await input.press("Tab");
      expect((await calls(page, "save_settings")).length, `typed ${v}`).toBe(before);
    }
  });

  test("the snapshot's text size sets --text-scale", async ({ page }) => {
    await open(page, snapshot({ settings: { textScale: 150 } }));
    const scale = () => page.evaluate(() => getComputedStyle(document.body).getPropertyValue("--text-scale").trim());
    expect(await scale()).toBe("1.5");
    await emit(page, "snapshot", snapshot({ settings: { textScale: 80 } }));
    await expect.poll(scale).toBe("0.8");
  });
});
