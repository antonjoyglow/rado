import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// title.js is a classic content script with no exports; load its functions without running main().
const source = readFileSync(new URL("./title.js", import.meta.url), "utf8");
const { radoTabTitle, prPath } = new Function(`${source}; return { radoTabTitle, prPath };`)();

describe("radoTabTitle", () => {
  it.each([
    ["/contoso/WebApp/_git/WebApp/pullrequest/3353", "#rado=ABC-4827&group", "WebApp:ABC-4827"],
    ["/contoso/Core/_git/Core/pullrequest/3059", "#rado=feature%2Fsearch-fix", "Core:feature/search-fix"],
    ["/o/p/_git/Tools.Scripts/pullrequest/7/files", "#rado=main", "Tools.Scripts:main"],
    ["/o/p/_git/WebApp/pullrequest/3353", "", null],
    ["/o/p/_git/WebApp/pullrequest/3353", "#other=1", null],
    ["/o/p/_git/WebApp", "#rado=ABC-1", null],
    ["/o/p/_git/WebApp/pullrequest/3353", "#rado=%E0%A4%A", null],
    ["/antonjoyglow/rado/pull/3", "#rado=feat%2Fx&group", "rado:feat/x"],
    ["/antonjoyglow/rado/pull/3/files", "#rado=main", "rado:main"],
    ["/antonjoyglow/rado/pulls", "#rado=main", null],
    ["/antonjoyglow/rado/issues/3", "#rado=main", null],
  ])("%s %s -> %s", (pathname, hash, title) => expect(radoTabTitle(pathname, hash)).toBe(title));
});

describe("prPath", () => {
  it("prSubPage_mapsToThePrRoot", () =>
    expect(prPath("/o/p/_git/WebApp/pullrequest/3353/files")).toBe("/o/p/_git/WebApp/pullrequest/3353"));
  it("githubPrSubPage_mapsToThePrRoot", () => expect(prPath("/o/rado/pull/3/files")).toBe("/o/rado/pull/3"));
  it("nonPrPage_isNull", () => expect(prPath("/o/p/_git/WebApp")).toBeNull());
});
