import { test, expect } from "@playwright/test";
import { open, calls, setFail, clearFail } from "./harness.js";
import { snapshot, item, member } from "./fixtures.js";

test("the list renders a row with its title and the tab count", async ({ page }) => {
  await open(page, snapshot({ items: [item({ id: 1, title: "Fix login" })] }));
  await expect(page.locator(".row .title")).toHaveText("Fix login");
  await expect(page.locator("#tab-needsReview .n")).toHaveText("1");
});

test.describe("Draft ready chip", () => {
  test("a linked draft is a button titled with its host and opens the draft, not the row", async ({ page }) => {
    const m = member({ id: 7, number: 7, agentDraft: { host: "github.com" } });
    await open(page, snapshot({ items: [item({ members: [m], id: 7 })] }));
    const chip = page.locator("button.agent-draft");
    await expect(chip).toHaveText("Draft ready");
    await expect(chip).toHaveAttribute("title", /github\.com/);
    await chip.click();
    expect((await calls(page, "open_draft")).map((c) => c.args)).toEqual([{ id: 7 }]);
    expect(await calls(page, "open_row")).toHaveLength(0);
  });

  test("an unlinked draft is plain text, not a button", async ({ page }) => {
    const m = member({ id: 8, agentDraft: { host: null } });
    await open(page, snapshot({ items: [item({ members: [m], id: 8 })] }));
    await expect(page.locator("span.agent-draft")).toHaveText("Draft ready");
    await expect(page.locator("button.agent-draft")).toHaveCount(0);
  });

  test("a failed open flashes Failed, says why in the title and doesn't open the row", async ({ page }) => {
    const m = member({ id: 9, agentDraft: { host: "dev.azure.com" } });
    await open(page, snapshot({ items: [item({ members: [m], id: 9 })] }));
    await setFail(page, "open_draft", "No draft link for this PR");
    const chip = page.locator("button.agent-draft");
    await chip.click();
    await expect(chip).toHaveText("Failed");
    await expect(chip).toHaveAttribute("title", /Couldn't open it: No draft link/);
    await expect(chip).toHaveText("Draft ready", { timeout: 3000 });
    expect(await calls(page, "open_row")).toHaveLength(0);
  });

  test("in an aggregate row the chip follows only the drafted member", async ({ page }) => {
    const a = member({ id: 11, number: 11, agentDraft: { host: "github.com" } });
    const b = member({ id: 12, number: 12, repo: "api" });
    await open(page, snapshot({ items: [item({ members: [a, b], id: 11, key: "agg" })] }));
    expect(await page.locator(".row .members > *").allTextContents()).toEqual(["web !11", "Draft ready", "api !12"]);
  });

  test("repeated failures don't keep growing the title", async ({ page }) => {
    const m = member({ id: 19, agentDraft: { host: "github.com" } });
    await open(page, snapshot({ items: [item({ members: [m], id: 19 })] }));
    await setFail(page, "open_draft", "boom");
    const chip = page.locator("button.agent-draft");
    for (let i = 0; i < 3; i++) {
      await chip.click();
      await expect(chip).toHaveText("Draft ready", { timeout: 3000 });
    }
    const title = await chip.getAttribute("title");
    expect(title.split("Couldn't open it").length - 1).toBe(1);
  });
});

test.describe("row actions", () => {
  test("Copy on a hovered row calls copy_row, flashes Copied and doesn't open the row", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 21, key: "k21" })] }));
    const row = page.locator(".row").first();
    await row.hover();
    const copy = row.locator(".actions button").first();
    await expect(copy).toBeVisible();
    await copy.click();
    await expect(copy).toHaveText("Copied");
    expect((await calls(page, "copy_row")).map((c) => c.args)).toEqual([{ key: "k21" }]);
    expect(await calls(page, "open_row")).toHaveLength(0);
  });

  test("Not mine on a hovered row hides it and doesn't open the row", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 22, key: "k22" })] }));
    const row = page.locator(".row").first();
    await row.hover();
    await row.locator(".actions button").nth(1).click();
    expect((await calls(page, "set_hidden")).map((c) => c.args)).toEqual([{ key: "k22", hidden: true }]);
    expect(await calls(page, "open_row")).toHaveLength(0);
  });

  test("actions are invisible without hover or focus", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 23 })] }));
    await page.mouse.move(0, 0);
    await expect(page.locator(".row .actions").first()).toHaveCSS("opacity", "0");
  });

  test("a failed copy flashes Failed, then Copy again", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 24 })] }));
    await setFail(page, "copy_row", "Clipboard busy");
    const row = page.locator(".row").first();
    await row.hover();
    const copy = row.locator(".actions button").first();
    await copy.click();
    await expect(copy).toHaveText("Failed");
    await expect(copy).toHaveText("Copy", { timeout: 3000 });
  });
});

test.describe("status line errors", () => {
  test("a failed open shows the error and announces it", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 31, key: "k31" })] }));
    await setFail(page, "open_row", "Couldn't open the browser: Edge not found");
    await page.locator(".row .title").first().click();
    await expect(page.locator(".status .failed")).toHaveText("Couldn't open the browser: Edge not found");
    await expect(page.locator(".status")).toBeVisible();
    await expect(page.locator('[role="status"].visually-hidden')).toHaveText(/Edge not found/);
  });

  test("the next successful action clears the error", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 32, key: "k32" })] }));
    await setFail(page, "open_row", "first failure");
    await page.locator(".row .title").first().click();
    await expect(page.locator(".status .failed")).toHaveText("first failure");
    await clearFail(page, "open_row");
    await page.locator(".row .title").first().click();
    await expect(page.locator(".status .failed")).toHaveText("");
    await expect(page.locator(".status")).toBeHidden();
  });

  test("a slow earlier failure doesn't overwrite a newer action's result", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 33, key: "k33" })] }));
    await page.evaluate(() => {
      let n = 0;
      window.__QA.handlers.open_row = () => (++n === 1 ? new Promise((_, rej) => setTimeout(() => rej("late failure"), 500)) : null);
    });
    await page.locator(".row .title").first().click();
    await page.locator(".row .title").first().click();
    await page.waitForTimeout(800);
    await expect(page.locator(".status .failed")).toHaveText("");
  });

  test("a failed Not mine shows the backend's text", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 34, key: "k34" })] }));
    await setFail(page, "set_hidden", "Changed, but not saved: disk full");
    const row = page.locator(".row").first();
    await row.hover();
    await row.locator(".actions button").nth(1).click();
    await expect(page.locator(".status .failed")).toHaveText("Changed, but not saved: disk full");
  });

  test("a failed workspace switch reports; opening Settings clears it", async ({ page }) => {
    const ws = (id, name) => ({ id, name, azureDevOps: { organization: "o", teams: [] }, github: { repos: [] }, tracker: { kind: "none" } });
    await open(page, snapshot({ items: [item({ id: 35 })], settings: { workspaces: [ws("a", "A"), ws("b", "B")], activeWorkspace: "a" } }));
    await setFail(page, "switch_workspace", "Couldn't save the switch: access denied");
    await page.keyboard.press("Control+ArrowRight");
    await expect(page.locator(".status .failed")).toHaveText("Couldn't save the switch: access denied");
    await page.getByTitle("Settings").click();
    await expect(page.locator(".status .failed")).toHaveText("");
  });

  test("a failed refresh is reported", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 36 })] }));
    await setFail(page, "refresh_now", "Already refreshing");
    await page.getByRole("button", { name: "Refresh now" }).click();
    await expect(page.locator(".status .failed")).toHaveText("Already refreshing");
  });

  test("the error clears when the window hides", async ({ page }) => {
    await open(page, snapshot({ items: [item({ id: 37 })] }));
    await setFail(page, "open_row", "boom");
    await page.locator(".row .title").first().click();
    await expect(page.locator(".status .failed")).toHaveText("boom");
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
      document.dispatchEvent(new Event("visibilitychange"));
      Object.defineProperty(document, "hidden", { configurable: true, get: () => false });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await expect(page.locator(".status .failed")).toHaveText("");
  });
});
