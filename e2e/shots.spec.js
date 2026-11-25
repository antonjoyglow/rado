import { test } from "@playwright/test";
import { open } from "./harness.js";
import { snapshot, item, member } from "./fixtures.js";

// Screenshots for looking at, not assertions: `npm run shots` writes them to shots/ and re-shoots the README's list.
test.skip(process.env.SHOTS !== "1", "run with npm run shots");

const iso = (hours) => new Date(Date.now() - hours * 3600_000).toISOString();

/** Made-up people, repos and tickets, shaped like the README's list. */
function rows() {
  const row = (id, title, author, tier, extra, members) => {
    const m = members ?? [member({ id, number: id, repo: "Core" })];
    return item({ key: `r${id}`, id, title, author, tier, members: m, ...extra });
  };
  const pr = (id, repo) => member({ id, number: id, repo });
  return [
    row(4350, "fix: retry the token refresh once before signing out", "Mira Holt", "urgent",
      { required: true, comments: { total: 2, openThreads: 1 }, createdAt: iso(240), updatedAt: iso(12) }),
    row(4239, "ENG-179: hide the beta branch from the user settings tree", "Jonas Weber", "normal",
      { required: false, ticket: "ENG-179", comments: { total: 4, openThreads: 2 }, createdAt: iso(312), updatedAt: iso(120) }),
    row(4412, "ENG-281: incident form adjustments (points 1–6)", "Ana Ruiz", "normal",
      { required: false, ticket: "ENG-281", branch: "ENG-281", comments: { total: 19, openThreads: 9 }, createdAt: iso(26), updatedAt: iso(3) },
      [pr(4412, "Web"), pr(4411, "Core")]),
    row(4420, "ENG-220: a live layer gets only its own filters (the sub-filter broke the live session)", "Ana Ruiz", "normal",
      { required: false, ticket: "ENG-220", branch: "ENG-220", approvals: 1, comments: { total: 12, openThreads: 0 }, createdAt: iso(30), updatedAt: iso(3) },
      [pr(4420, "Web"), pr(4419, "Core")]),
    row(4401, "chore: bump the map tiles cache to 30 days", "Lena Ortiz", "normal",
      { required: false, createdAt: iso(50), updatedAt: iso(20) }),
    row(4377, "feat(api): page the audit log by cursor", "Tomas Berg", "low",
      { required: false, comments: { total: 1, openThreads: 0 }, createdAt: iso(100), updatedAt: iso(60) }),
  ];
}

const shoot = async (page, theme, path) => {
  await open(page, snapshot({ items: rows(), settings: { theme } }));
  await page.locator(".row").first().waitFor();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path });
};

for (const theme of ["dark", "light"]) {
  for (const [size, width] of [["wide", 1135], ["narrow", 480]]) {
    test(`${theme} ${size}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await shoot(page, theme, `shots/list-${theme}-${size}.png`);
    });
  }
}

test.describe("README", () => {
  test.use({ viewport: { width: 640, height: 447 }, deviceScaleFactor: 2 });
  test("the review list", async ({ page }) => {
    await shoot(page, "dark", "docs/screenshots/review-list.png");
  });
});
