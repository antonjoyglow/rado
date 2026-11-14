import { describe, expect, it } from "vitest";
import type { ReviewItem } from "./api";
import { defaultListTab, restoreTab, SECTIONS, stepTab } from "./tabs";

const item = (over: Partial<ReviewItem>): ReviewItem => ({
  key: "pr:1", id: 1, provider: "ado", number: 1, title: "t", repo: "r", branch: "feature/x", members: [], newestAgeHours: 1, ticket: null, author: "a", target: "dev", releaseTarget: false, ageHours: 1,
  createdAt: "2026-11-04T11:00:00Z", updatedAt: null,
  required: false, draft: false, state: "needsReview", score: 50, tier: "normal", hidden: false, approvals: 0, objections: 0, comments: null,
  actionable: true,
  ...over,
});

describe("defaultListTab", () => {
  it("reviewsWaiting_isReview", () => {
    expect(defaultListTab([item({ state: "mine" }), item({ state: "needsReview" })])).toBe("needsReview");
  });

  it("onlyReReviews_isReReview", () => {
    expect(defaultListTab([item({ state: "mine" }), item({ state: "reReview" })])).toBe("reReview");
  });

  it("onlyYourPrs_isMine", () => {
    expect(defaultListTab([item({ state: "approved" }), item({ state: "mine" })])).toBe("mine");
  });

  it("hiddenReviewOnly_isNotCountedAsReview", () => {
    expect(defaultListTab([item({ state: "needsReview", hidden: true }), item({ state: "mine" })])).toBe("mine");
  });

  it("nothingInTheFirstThree_fallsBackToReview", () => {
    expect(defaultListTab([item({ state: "stale" }), item({ state: "waitingOnAuthor" })])).toBe("needsReview");
    expect(defaultListTab([])).toBe("needsReview");
  });
});

describe("restoreTab", () => {
  const keys = SECTIONS.map((s) => s.key);

  it("knownKey_isKept", () => {
    expect(restoreTab("stale", keys)).toBe("stale");
  });

  it("unknownOrMissing_isNull", () => {
    expect(restoreTab("collapsed", keys)).toBeNull();
    expect(restoreTab(null, keys)).toBeNull();
  });
});

describe("stepTab", () => {
  it("arrows_moveAndWrap", () => {
    expect(stepTab(0, "ArrowRight", 7)).toBe(1);
    expect(stepTab(6, "ArrowRight", 7)).toBe(0);
    expect(stepTab(0, "ArrowLeft", 7)).toBe(6);
    expect(stepTab(3, "ArrowLeft", 7)).toBe(2);
  });

  it("homeAndEnd_goToTheEnds", () => {
    expect(stepTab(3, "Home", 7)).toBe(0);
    expect(stepTab(3, "End", 7)).toBe(6);
  });

  it("otherKeys_areIgnored", () => {
    expect(stepTab(3, "Enter", 7)).toBeNull();
    expect(stepTab(3, "Escape", 7)).toBeNull();
  });
});
