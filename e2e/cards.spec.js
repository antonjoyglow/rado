import { test, expect } from "@playwright/test";
import { open } from "./harness.js";
import { snapshot, item, member, worstCase } from "./fixtures.js";

const TRANSPARENT = "rgba(0, 0, 0, 0)";

test("every fact and chip of a row has its own look", async ({ page }) => {
  await open(page, snapshot({ items: [worstCase()] }));
  const facts = page.locator(".row .meta > span");
  expect(await facts.evaluateAll((els) => els.map((e) => [e.className, e.textContent]))).toEqual([
    ["next-label", "Review next"],
    ["fact-tag", "2 PRs"],
    ["fact-branch", "feature/x"],
    ["", "Ada Lovelace"],
    ["fact-required", "required"],
    ["fact-approved", "✓ 2"],
    ["fact-objected", "✗ 1"],
    ["fact-comments", "3 comments"],
    ["fact-tag", "1 open"],
    ["fact-tag", "draft"],
    ["fact-tag", "→ main"],
  ]);
  const filled = await page.locator(".row .meta > span:is(.fact-tag, .fact-required, .fact-approved, .fact-objected, .next-label)")
    .evaluateAll((els) => els.map((e) => getComputedStyle(e).backgroundColor));
  expect(filled).toHaveLength(8);
  for (const colour of filled) expect(colour).not.toBe(TRANSPARENT);

  const chip = (selector) => page.locator(`.row .members ${selector}`).evaluate((e) => getComputedStyle(e).backgroundColor);
  expect(await chip(".ticket")).not.toBe(TRANSPARENT);
  expect(await chip('[data-assigned="false"]')).toBe(TRANSPARENT);
  const accent = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--accent").trim());
  expect(await chip("button.agent-draft")).toBe(await page.evaluate((a) => {
    const probe = document.body.appendChild(Object.assign(document.createElement("i"), { style: `background:${a}` }));
    return getComputedStyle(probe).backgroundColor;
  }, accent));
});

test("a wide window puts the chips beside the details; a narrow one under them", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 700 });
  await open(page, snapshot({ items: [worstCase()] }));
  const box = (s) => page.locator(`.row ${s}`).first().boundingBox();
  const [meta, members] = [await box(".meta"), await box(".members")];
  expect(members.x).toBeGreaterThanOrEqual(meta.x + meta.width);

  await page.setViewportSize({ width: 500, height: 700 });
  const [meta2, members2] = [await box(".meta"), await box(".members")];
  expect(members2.y).toBeGreaterThanOrEqual(meta2.y + meta2.height);
});

for (const width of [740, 1100]) {
  test(`at 150% text and ${width} px, many chips leave the title room`, async ({ page }) => {
    await page.setViewportSize({ width, height: 700 });
    const members = Array.from({ length: 6 }, (_, i) => member({ id: 60 + i, number: 4400 + i, repo: `TacticalCore${i}` }));
    await open(page, snapshot({ items: [item({ members, id: 60, key: "many", ticket: "TAC-5281" })], settings: { textScale: 150 } }));
    const title = await page.locator(".row .title").boundingBox();
    expect(title.width).toBeGreaterThanOrEqual(200);
  });
}

test("the wide layout starts later with larger text", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await open(page, snapshot({ items: [worstCase()], settings: { textScale: 150 } }));
  const [meta, members] = [await page.locator(".row .meta").boundingBox(), await page.locator(".row .members").boundingBox()];
  expect(members.y).toBeGreaterThanOrEqual(meta.y + meta.height);
});

for (const width of [500, 1100]) {
  test(`40 rows scroll, and a one-line card with its gap is at most 96 px at ${width} px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const items = Array.from({ length: 40 }, (_, i) => item({ id: 100 + i, key: `r${i}`, title: "Fix login" }));
    await open(page, snapshot({ items }));
    await expect(page.locator(".row")).toHaveCount(40);
    const heights = await page.locator(".row").evaluateAll((els) => els.map((e) => e.getBoundingClientRect().height));
    for (const h of heights) expect(h + 8).toBeLessThanOrEqual(96);
    expect(await page.locator(".body").evaluate((b) => b.scrollHeight > b.clientHeight)).toBe(true);
  });
}

test("the end of the list says how many and when they were checked", async ({ page }) => {
  await open(page, snapshot({ items: [item({ id: 41, key: "a" }), item({ id: 42, key: "b" })] }));
  await expect(page.locator(".body > .end .end-count")).toHaveText("That's all 2");
  await expect(page.locator(".body > .end")).toContainText("checked just now");
});

test("an empty tab shows no end marker", async ({ page }) => {
  await open(page, snapshot({ items: [] }));
  await expect(page.locator(".body > .empty")).toBeVisible();
  await expect(page.locator(".body > .end")).toHaveCount(0);
});

test("Settings replace the list", async ({ page }) => {
  await open(page, snapshot());
  await page.locator('header [title="Settings"]').click();
  await expect(page.locator(".settings")).toBeVisible();
  await expect(page.locator(".body")).toBeHidden();
});

test("in forced colours the cards and pills keep borders", async ({ page }) => {
  await page.emulateMedia({ forcedColors: "active" });
  await open(page, snapshot({ items: [worstCase()] }));
  for (const selector of [".row", ".row .meta .fact-tag", ".row .member"]) {
    expect(await page.locator(selector).first().evaluate((e) => getComputedStyle(e).borderTopStyle), selector).not.toBe("none");
  }
});
