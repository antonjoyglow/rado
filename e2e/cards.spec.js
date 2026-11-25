import { test, expect } from "@playwright/test";
import { open } from "./harness.js";
import { snapshot, item, member } from "./fixtures.js";

const TRANSPARENT = "rgba(0, 0, 0, 0)";

/** Every fact `rowFacts` can show, a ticket, a PR not assigned to you and a linked agent draft. */
function worstCase() {
  const mine = member({ id: 31, number: 31, agentDraft: { host: "github.com" } });
  const other = member({ id: 32, number: 32, repo: "api", state: null });
  return item({
    members: [mine, other], id: 31, key: "worst", ticket: "TAC-1", required: true, approvals: 2, objections: 1,
    comments: { total: 3, openThreads: 1 }, draft: true, releaseTarget: true, target: "main",
  });
}

test("every fact and chip of a row has its own look", async ({ page }) => {
  await open(page, snapshot({ items: [worstCase()] }));
  const facts = page.locator(".row .meta > span");
  expect(await facts.evaluateAll((els) => els.map((e) => [e.className, e.textContent]))).toEqual([
    ["next-label", "Review next"],
    ["tag", "2 PRs"],
    ["branch", "feature/x"],
    ["", "Ada Lovelace"],
    ["required", "required"],
    ["approved", "✓ 2"],
    ["objected", "✗ 1"],
    ["comments", "3 comments"],
    ["tag", "1 open"],
    ["tag", "draft"],
    ["tag", "→ main"],
  ]);
  const filled = await page.locator(".row .meta > span:is(.tag, .required, .approved, .objected, .next-label)")
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

test("the focused selected tab's ring differs from its fill", async ({ page }) => {
  await open(page, snapshot());
  await page.locator("#tab-needsReview").focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowLeft");
  const { outline, fill } = await page.locator("#tab-needsReview").evaluate((e) => {
    const s = getComputedStyle(e);
    return { outline: s.outlineColor, fill: s.backgroundColor };
  });
  expect(outline).not.toBe(fill);
});

/** WCAG contrast of two computed colours, `rgb(…)` or `color(srgb …)`. */
function contrast(a, b) {
  const channels = (c) => {
    const n = c.match(/[\d.]+/g).map(Number);
    return c.startsWith("color(") ? n.slice(0, 3) : n.slice(0, 3).map((v) => v / 255);
  };
  const lum = (c) => {
    const [r, g, bl] = channels(c).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const PAIRS = [
  ...["--ground", "--card", "--card-hover", "--tag", "--tint-accent", "--tint-normal", "--next-bg"].map((f) => ["--fg", f]),
  ...["--ground", "--card", "--card-hover", "--next-bg"].map((f) => ["--muted", f]),
  ["--on-accent", "--accent"],
];

for (const theme of ["light", "dark", "highContrast", "custom"]) {
  test(`every text colour reads at 4.5:1 on the fill under it in the ${theme} theme`, async ({ page }) => {
    await open(page, snapshot({ settings: { theme } }));
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const colours = await page.evaluate((names) => Object.fromEntries(names.map((n) => {
      const probe = document.body.appendChild(Object.assign(document.createElement("i"), { style: `color:var(${n})` }));
      return [n, getComputedStyle(probe).color];
    })), [...new Set(PAIRS.flat())]);
    for (const [text, fill] of PAIRS) {
      expect(contrast(colours[text], colours[fill]), `${text} on ${fill}: ${colours[text]} on ${colours[fill]}`).toBeGreaterThanOrEqual(4.5);
    }
  });
}

test("in forced colours the cards and pills keep borders", async ({ page }) => {
  await page.emulateMedia({ forcedColors: "active" });
  await open(page, snapshot({ items: [worstCase()] }));
  for (const selector of [".row", ".row .meta .tag", ".row .member"]) {
    expect(await page.locator(selector).first().evaluate((e) => getComputedStyle(e).borderTopStyle), selector).not.toBe("none");
  }
});
