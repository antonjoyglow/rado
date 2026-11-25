import { test, expect } from "@playwright/test";
import { open, calls, setFail, clearFail, setDelay, emit, settle } from "./harness.js";
import { snapshot, item, workspace } from "./fixtures.js";

const openSettings = async (page, snap = snapshot()) => {
  await open(page, snap);
  await page.getByTitle("Settings").click();
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
};
const pat = (page) => page.getByLabel("PAT", { exact: true });
const ghToken = (page) => page.getByLabel("GitHub token", { exact: true });

test.describe("pasted tokens", () => {
  test("Enter saves the PAT once, empties the field and says it's saved", async ({ page }) => {
    await openSettings(page);
    await pat(page).fill("  secret-pat-1  ");
    await pat(page).press("Enter");
    await expect(page.getByText("Saved to Windows Credential Manager")).toBeVisible();
    expect((await calls(page, "save_pat")).map((c) => c.args)).toEqual([{ workspace: "ws1", pat: "  secret-pat-1  " }]);
    await expect(pat(page)).toHaveValue("");
    await expect(pat(page)).toHaveAttribute("placeholder", /saved/);
  });

  test("Enter and Save during a slow save still save once", async ({ page }) => {
    await openSettings(page);
    await setDelay(page, "save_pat", 400);
    await pat(page).fill("secret-pat-2");
    await pat(page).press("Enter");
    await page.getByRole("button", { name: "Save" }).first().click();
    await settle(page, "save_pat");
    expect(await calls(page, "save_pat")).toHaveLength(1);
  });

  test("Esc leaves Settings and saves the pasted PAT", async ({ page }) => {
    await openSettings(page);
    await pat(page).fill("secret-pat-3");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("heading", { name: "Reviews" })).toBeVisible();
    await expect.poll(async () => (await calls(page, "save_pat")).map((c) => c.args.pat)).toEqual(["secret-pat-3"]);
    expect(await calls(page, "hide_list")).toHaveLength(0);
  });

  test("the back button leaves Settings and saves the pasted GitHub token", async ({ page }) => {
    await openSettings(page, snapshot({ hasGithubToken: false }));
    await ghToken(page).fill("ghp_secret4");
    await page.getByTitle("Back to reviews").click();
    await expect.poll(async () => (await calls(page, "save_github_token")).map((c) => c.args)).toEqual([{ workspace: "ws1", token: "ghp_secret4" }]);
  });

  test("a save that fails on the way out reaches the status line", async ({ page }) => {
    await openSettings(page);
    await setFail(page, "save_pat", "Credential Manager: access denied");
    await pat(page).fill("secret-pat-5");
    await page.keyboard.press("Escape");
    await expect(page.locator(".status .failed")).toHaveText(/Credential Manager: access denied/);
  });

  test("a failed save keeps the value, and Enter retries", async ({ page }) => {
    await openSettings(page);
    await setFail(page, "save_pat", "Credential Manager: busy");
    await pat(page).fill("secret-pat-6");
    await pat(page).press("Enter");
    await expect(page.getByText("Credential Manager: busy")).toBeVisible();
    await expect(pat(page)).toHaveValue("secret-pat-6");
    await clearFail(page, "save_pat");
    await pat(page).press("Enter");
    await expect(page.getByText("Saved to Windows Credential Manager")).toBeVisible();
  });

  test("Save with an empty field asks to paste first; whitespace isn't sent", async ({ page }) => {
    await openSettings(page, snapshot({ saved: { ws1: { pat: false, githubToken: false } } }));
    await page.getByRole("button", { name: "Save" }).first().click();
    await expect(page.getByText("Paste the PAT first")).toBeVisible();
    await pat(page).fill("    ");
    await pat(page).press("Enter");
    expect(await calls(page, "save_pat")).toHaveLength(0);
  });

  test("a snapshot arriving while typing doesn't wipe the pasted token", async ({ page }) => {
    const snap = snapshot();
    await openSettings(page, snap);
    await pat(page).fill("secret-pat-9");
    await emit(page, "snapshot", { ...snap, lastRefreshed: new Date().toISOString() });
    await expect(pat(page)).toHaveValue("secret-pat-9");
  });
});

test.describe("organization field", () => {
  test("a refused link shows the reason by the field and reverts it", async ({ page }) => {
    await openSettings(page);
    await setFail(page, "save_workspace", "That link has no organization in it");
    await page.getByLabel("Organization").fill("  https://dev.azure.com/  ");
    await page.getByLabel("Organization").press("Tab");
    await expect(page.getByText("That link has no organization in it")).toBeVisible();
    expect((await calls(page, "save_workspace")).map((c) => c.args.workspace.azureDevOps.organization)).toEqual(["https://dev.azure.com/"]);
    await expect(page.getByLabel("Organization")).toHaveValue("contoso");
  });

  test("after a save the field keeps the pasted link", async ({ page }) => {
    await openSettings(page);
    await page.evaluate(() => {
      window.__e2e.handlers.save_workspace = ({ workspace }) => {
        const s = window.__e2e.responses.get_snapshot;
        s.settings.workspaces[0] = { ...workspace, azureDevOps: { ...workspace.azureDevOps, organization: "fabrikam" } };
        setTimeout(() => {
          window.__e2e.emit("snapshot", s);
          window.__e2e.snapshotSent = true;
        }, 10);
        return null;
      };
    });
    await page.getByLabel("Organization").fill("https://dev.azure.com/fabrikam/Proj/_git/web");
    await page.getByLabel("Organization").press("Tab");
    await page.waitForFunction(() => window.__e2e.snapshotSent);
    await expect(page.getByLabel("Organization")).toHaveValue("https://dev.azure.com/fabrikam/Proj/_git/web");
  });
});

test.describe("save queue", () => {
  test("rapid edits to one workspace save in order and the last value wins", async ({ page }) => {
    await openSettings(page);
    await setDelay(page, "save_workspace", 150);
    const teams = page.getByLabel("My teams");
    for (const v of ["A", "A, B", "A, B, C"]) {
      await teams.fill(v);
      await teams.press("Tab");
    }
    await expect.poll(async () => (await calls(page, "save_workspace")).at(-1)?.args.workspace.azureDevOps.teams.join("|")).toBe("A|B|C");
    await settle(page, "save_workspace");
    const sent = (await calls(page, "save_workspace")).map((c) => c.args.workspace.azureDevOps.teams.join("|"));
    // Queued saves send the workspace as it is when their turn comes, so they coalesce to the latest value.
    expect(sent[0]).toBe("A");
    expect(sent.at(-1)).toBe("A|B|C");
  });

  test("a failed save drops the queued ones and reverts the fields", async ({ page }) => {
    await openSettings(page);
    await page.evaluate(() => {
      let n = 0;
      window.__e2e.handlers.save_workspace = () => new Promise((res, rej) => setTimeout(() => (++n === 1 ? rej("disk full") : res(null)), 200));
    });
    await page.getByLabel("My teams").fill("T1");
    await page.getByLabel("My teams").press("Tab");
    await page.getByLabel("Watched repos").fill("o/r");
    await page.getByLabel("Watched repos").press("Tab");
    await expect(page.getByText("disk full")).toBeVisible();
    await settle(page, "save_workspace");
    expect((await calls(page, "save_workspace")).length).toBe(1);
    await expect(page.getByText(/an earlier change failed to save/)).toBeVisible();
    await expect(page.getByLabel("My teams")).toHaveValue("");
  });

  test("Settings reopened during a slow save queues behind it", async ({ page }) => {
    await openSettings(page);
    await setDelay(page, "save_workspace", 500);
    await page.getByLabel("Workspace name").fill("First");
    await page.getByLabel("Workspace name").press("Tab");
    await page.keyboard.press("Escape");
    await page.getByTitle("Settings").click();
    await expect(page.getByLabel("Workspace name")).toHaveValue("First");
    await page.getByLabel("Workspace name").fill("Second");
    await page.getByLabel("Workspace name").press("Tab");
    await expect.poll(async () => (await calls(page, "save_workspace")).map((c) => c.args.workspace.name)).toEqual(["First", "Second"]);
  });

  test("an empty workspace name is refused and nothing is sent", async ({ page }) => {
    await openSettings(page);
    await page.getByLabel("Workspace name").fill("   ");
    await page.getByLabel("Workspace name").press("Tab");
    await expect(page.getByText("A workspace needs a name.")).toBeVisible();
    expect(await calls(page, "save_workspace")).toHaveLength(0);
  });
});

test.describe("empty list", () => {
  test("an empty Review tab explains which PRs are listed, with no link without a GitHub token", async ({ page }) => {
    await open(page, snapshot({ items: [] }));
    await expect(page.locator(".body .empty")).toHaveText("Nothing here. Rado lists PRs where you or your teams are reviewers.");
    await expect(page.getByRole("button", { name: "Watched repos" })).toHaveCount(0);
  });

  test("with a GitHub token the Watched repos link opens Settings with the field focused", async ({ page }) => {
    await open(page, snapshot({ items: [], hasGithubToken: true }));
    await page.getByRole("button", { name: "Watched repos" }).click();
    await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
    await expect(page.getByLabel("Watched repos")).toBeFocused();
  });

  test("before the first refresh it says Loading", async ({ page }) => {
    await open(page, snapshot({ items: [], lastRefreshed: null, polling: true }));
    await expect(page.locator(".body .empty")).toHaveText("Loading your reviews…");
  });

  test("with only own PRs the Review tab explains", async ({ page }) => {
    await open(page, snapshot({ items: [item({ state: "mine", actionable: false })] }));
    await page.locator("#tab-needsReview").click();
    await expect(page.locator(".body .empty")).toContainText("Rado lists PRs where you or your teams are reviewers.");
  });

  test("an empty Review tab while Re-review has rows has no explanation", async ({ page }) => {
    await open(page, snapshot({ items: [item({ state: "reReview" })] }));
    await page.locator("#tab-needsReview").click();
    await expect(page.locator(".body .empty")).toHaveText("Nothing here.");
  });

  test("the Watched repos link selects the active workspace in Settings", async ({ page }) => {
    const ws = [workspace("a", "A"), workspace("b", "B")];
    await open(page, snapshot({ items: [], hasGithubToken: true, settings: { workspaces: ws, activeWorkspace: "b" } }));
    await page.getByRole("button", { name: "Watched repos" }).click();
    await expect(page.locator('.ws-item[aria-pressed="true"]')).toContainText("B");
  });
});

test.describe("PocketBook pairing", () => {
  /** Opens Settings → General with `responses` as the pairing commands' answers. */
  const general = async (page, responses = {}) => {
    await open(page, snapshot());
    await page.evaluate((r) => Object.assign(window.__e2e.responses, r), responses);
    await page.getByTitle("Settings").click();
    await page.locator("#settings-tab-general").click();
  };
  const paired = { paired: true, address: "192.168.1.14:7878", problem: null };

  test("pairing shows the token as dots and the address, and Copy config goes through Rado", async ({ page }) => {
    await general(page, { pair_ink: paired });
    await expect(page.getByText("Laptop address: 192.168.1.14:7878")).toBeVisible();
    await page.getByRole("button", { name: "Pair a device" }).click();
    await expect(page.getByText("•••••••••••••••• saved")).toBeVisible();
    await page.getByRole("button", { name: "Copy config" }).click();
    await expect(page.getByText("Copied", { exact: true })).toBeVisible();
    expect(await calls(page, "copy_ink_config")).toHaveLength(1);
  });

  test("Replace makes a new token, Remove goes back to Pair, and a listener problem shows", async ({ page }) => {
    await general(page, {
      ink_status: { ...paired, problem: "Can't listen on 192.168.1.14:7878 (in use)" },
      pair_ink: paired,
      unpair_ink: { paired: false, address: null, problem: null },
    });
    await expect(page.getByText("Can't listen on 192.168.1.14:7878 (in use)")).toBeVisible();
    await page.getByRole("button", { name: "Replace token" }).click();
    await expect(page.getByText("Replaced")).toBeVisible();
    await page.getByRole("button", { name: "Remove" }).click();
    await expect(page.getByRole("button", { name: "Pair a device" })).toBeVisible();
    await expect(page.getByText("Laptop address: none, not on Wi-Fi")).toBeVisible();
    expect((await calls(page, "pair_ink")).length + (await calls(page, "unpair_ink")).length).toBe(2);
  });

  test("a failed pairing says why", async ({ page }) => {
    await general(page);
    await setFail(page, "pair_ink", "Credential Manager: access denied");
    await page.getByRole("button", { name: "Pair a device" }).click();
    await expect(page.getByText("Credential Manager: access denied")).toBeVisible();
  });
});

test("with no token at all Settings opens at start", async ({ page }) => {
  await open(page, snapshot({ items: [], hasPat: false, hasGithubToken: false, saved: { ws1: { pat: false, githubToken: false } } }));
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
});
