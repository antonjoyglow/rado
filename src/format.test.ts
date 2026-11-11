import { describe, expect, it } from "vitest";
import type { Member, ReviewItem } from "./api";
import { age, ageColumn, ago, datesTooltip, HEAT_CAP, prLabel, heat, newestFirst, nextReview, parseMinutes, parseScale, pillCounts, stepScale } from "./format";

const item = (over: Partial<ReviewItem>): ReviewItem => ({
  key: "pr:1", id: 1, provider: "ado", number: 1, title: "t", repo: "r", branch: "feature/x", members: [], newestAgeHours: 1, ticket: null, author: "a", target: "dev", ageHours: 1,
  createdAt: "2026-11-04T11:00:00Z", updatedAt: null,
  required: false, viaGroup: false, draft: false, state: "needsReview", score: 50, tier: "normal", hidden: false, approvals: 0, objections: 0, comments: null,
  actionable: over.state === undefined || over.state === "needsReview" || over.state === "reReview",
  ...over,
});

describe("nextReview", () => {
  const rows = [
    item({ key: "mine", state: "mine", actionable: true }),
    item({ key: "hidden", hidden: true }),
    item({ key: "stale", state: "stale" }),
    item({ key: "first" }),
    item({ key: "second", state: "reReview" }),
    item({ key: "approved", state: "approved" }),
  ];

  it("nothingOpened_isTheTopRankedReview", () => {
    expect(nextReview(rows, new Map())?.key).toBe("first");
  });

  it("topOneOpened_movesToTheNextReview", () => {
    expect(nextReview(rows, new Map([["first", 1000]]))?.key).toBe("second");
  });

  it("allOpened_isTheOneOpenedLongestAgo", () => {
    expect(nextReview(rows, new Map([["first", 2000], ["second", 1000]]))?.key).toBe("second");
  });

  it("noReviewWaiting_isNull", () => {
    expect(nextReview(rows.filter((r) => !["first", "second"].includes(r.key)), new Map())).toBeNull();
  });
});

describe("newestFirst", () => {
  it("rowsWithANewerPr_comeFirstEvenIfTheirOldestPrIsOlder", () => {
    const rows = [
      item({ key: "old", ageHours: 300, newestAgeHours: 300 }),
      item({ key: "aggregate", ageHours: 900, newestAgeHours: 2 }),
      item({ key: "week", ageHours: 170, newestAgeHours: 170 }),
    ];
    expect(newestFirst(rows).map((r) => r.key)).toEqual(["aggregate", "week", "old"]);
  });
});

describe("age", () => {
  it.each([[0, "new"], [5, "5h"], [47, "47h"], [48, "2d"], [100, "4d"]])("%i hours -> %s", (h, text) => {
    expect(age(h)).toBe(text);
  });
});

describe("ago", () => {
  const now = new Date("2026-11-04T12:00:00Z");
  it.each([
    [null, "never"],
    ["2026-11-04T11:59:30Z", "just now"],
    ["2026-11-04T11:57:00Z", "3 min ago"],
    ["2026-11-04T09:00:00Z", "3 h ago"],
  ])("%s -> %s", (iso, text) => {
    expect(ago(iso, now)).toBe(text);
  });
});

describe("pillCounts", () => {
  it("noItems_isZeroZero", () => expect(pillCounts([])).toEqual({ reviews: 0, mine: 0 }));
  it("onlyApprovedOrWaiting_isZeroZero", () =>
    expect(pillCounts([item({ state: "approved" }), item({ state: "waitingOnAuthor" })])).toEqual({ reviews: 0, mine: 0 }));
  it("reviewsToDo_countLeft", () =>
    expect(pillCounts([item({ state: "needsReview" }), item({ state: "reReview" })])).toEqual({ reviews: 2, mine: 0 }));
  it("myPrNeedingWork_countsRight", () =>
    expect(pillCounts([item({ state: "mine", actionable: true }), item({ state: "needsReview" })])).toEqual({ reviews: 1, mine: 1 }));
  it("myPrNotNeedingWork_isNotCounted", () => expect(pillCounts([item({ state: "mine" })])).toEqual({ reviews: 0, mine: 0 }));
  it("hiddenRows_areNotCounted", () =>
    expect(pillCounts([item({ hidden: true }), item({ state: "mine", actionable: true, hidden: true })])).toEqual({ reviews: 0, mine: 0 }));
});

describe("heat", () => {
  it.each([
    [0, "hsl(225, 100%, 58%)"],
    [5, "hsl(113, 100%, 58%)"],
    [HEAT_CAP, "hsl(0, 100%, 58%)"],
    [123, "hsl(0, 100%, 58%)"],
    [-1, "hsl(225, 100%, 58%)"],
  ])("%i -> %s", (count, colour) => expect(heat(count)).toBe(colour));
});

describe("parseScale", () => {
  it.each([["100", 100], [" 75 ", 75], ["110", 110], ["200", 200], ["74", null], ["201", null], ["87.5", null], ["", null]])(
    "%s -> %s",
    (text, value) => expect(parseScale(text)).toBe(value),
  );
});

describe("stepScale", () => {
  it.each([
    [100, 1, 125], [100, -1, 75], [110, 1, 125], [110, -1, 100], [200, 1, 200], [75, -1, 75], [190, 1, 200], [80, -1, 75],
  ] as const)("%i step %i -> %i", (current, direction, next) => expect(stepScale(current, direction)).toBe(next));
});

describe("ageColumn", () => {
  const now = new Date("2026-11-04T12:00:00Z");
  it("withActivity_showsOpenedAndUpdated", () =>
    expect(ageColumn("2026-10-30T12:00:00Z", "2026-11-04T10:00:00Z", now)).toEqual({ opened: "5d", updated: "2h" }));
  it("noActivity_showsOnlyOpened", () => expect(ageColumn("2026-11-04T09:00:00Z", null, now)).toEqual({ opened: "3h", updated: null }));
  it("activityWithinTheHour_isNowNotNew", () =>
    expect(ageColumn("2026-11-01T12:00:00Z", "2026-11-04T11:30:00Z", now)).toEqual({ opened: "3d", updated: "now" }));
  it("bothPartsUseTheSameClock_soActivityIsNeverOlderThanOpening", () => {
    const later = new Date("2026-11-04T15:00:00Z");
    expect(ageColumn("2026-11-04T09:00:00Z", "2026-11-04T09:30:00Z", later)).toEqual({ opened: "6h", updated: "5h" });
  });
});

describe("datesTooltip", () => {
  const member = (over: Partial<Member>): Member => ({
    id: 1, provider: "ado", number: 1, repo: "WebApp", title: "t", url: "u", state: "mine", createdAt: "C", updatedAt: "U", ...over,
  });
  it("listsEachPrWithItsTimes_relatedOrQuietPrsShowOnlyOpened", () =>
    expect(datesTooltip([member({}), member({ id: 2, number: 2, repo: "Core", state: null, updatedAt: null })], (iso) => `<${iso}>`)).toBe(
      "WebApp !1: opened <C> · last activity <U>\nCore !2: opened <C>",
    ));
});

describe("prLabel", () => {
  it.each([
    [{ repo: "WebApp", provider: "ado", number: 3353 }, "WebApp !3353"],
    [{ repo: "rado", provider: "github", number: 4 }, "rado #4"],
  ] as const)("%o -> %s", (pr, label) => expect(prLabel(pr)).toBe(label));
});

describe("parseMinutes", () => {
  it.each([["3", 3], [" 15 ", 15], ["120", 120], ["0", null], ["121", null], ["2.5", null], ["", null], ["abc", null]])(
    "%s -> %s",
    (text, value) => expect(parseMinutes(text)).toBe(value),
  );
});
