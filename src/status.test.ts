import { describe, expect, it } from "vitest";
import { counterProblem, glows, isStale, listActions, newReviewsText, onFocus, onFresh, onSnapshot, refreshTitle, showStatus, statusLine } from "./status";

const now = new Date("2026-11-17T12:00:00Z");
const minutesAgo = (m: number) => new Date(now.getTime() - m * 60_000).toISOString();
const polled = (over: Partial<{ polling: boolean; error: string | null; lastRefreshed: string | null; pollMinutes: number; workspace: string }> = {}) => ({
  polling: over.polling ?? false,
  error: over.error ?? null,
  lastRefreshed: over.lastRefreshed === undefined ? minutesAgo(1) : over.lastRefreshed,
  settings: { pollMinutes: over.pollMinutes ?? 5, activeWorkspace: over.workspace ?? "a" },
});

describe("isStale", () => {
  it("exactlyTwoPollsPlusAMinute_isNotStale", () => {
    expect(isStale(minutesAgo(11), 5, now)).toBe(false);
  });

  it("aMomentPastTwoPollsPlusAMinute_isStale", () => {
    expect(isStale(new Date(now.getTime() - 11 * 60_000 - 1).toISOString(), 5, now)).toBe(true);
  });

  it("neverRefreshed_isNotStale", () => {
    expect(isStale(null, 5, now)).toBe(false);
  });
});

describe("statusLine", () => {
  it("freshAndIdle_isNull", () => {
    expect(statusLine(polled(), null, now)).toBeNull();
  });

  it("stale_saysWhenLastChecked", () => {
    expect(statusLine(polled({ lastRefreshed: minutesAgo(12) }), null, now)).toEqual({ kind: "stale", text: "Last checked 12 min ago" });
  });

  it("pollingUnderFiveSeconds_isNull", () => {
    expect(statusLine(polled({ polling: true }), now.getTime() - 4_999, now)).toBeNull();
  });

  it("pollingFiveSeconds_isBusy", () => {
    expect(statusLine(polled({ polling: true }), now.getTime() - 5_000, now)).toEqual({ kind: "busy", text: "Refreshing… usually 1–3 s (5 s)" });
  });

  it("errorDuringAShortPoll_staysShown", () => {
    expect(statusLine(polled({ polling: true, error: "Offline" }), now.getTime() - 1_000, now)).toEqual({ kind: "error", text: "Offline" });
  });

  it("errorAndStale_showsTheErrorAndWhenLastChecked", () => {
    expect(statusLine(polled({ error: "Offline", lastRefreshed: minutesAgo(30) }), null, now)).toEqual({ kind: "error", text: "Offline · last checked 30 min ago" });
  });

  it("errorNeverRefreshed_showsJustTheError", () => {
    expect(statusLine(polled({ error: "Offline", lastRefreshed: null }), null, now)).toEqual({ kind: "error", text: "Offline" });
  });

  it("errorDuringAPollOfFiveSeconds_isBusy", () => {
    expect(statusLine(polled({ polling: true, error: "Offline" }), now.getTime() - 5_000, now)?.kind).toBe("busy");
  });

  it("pollingWithoutAStartTime_isNull", () => {
    expect(statusLine(polled({ polling: true }), null, now)).toBeNull();
  });
});

describe("showStatus", () => {
  it("nothingToSay_isHidden", () => {
    expect(showStatus(null, null, null, null)).toBe(false);
  });

  it.each([
    [{ kind: "stale" as const, text: "x" }, null, null, null],
    [null, "Settings could not be read", null, null],
    [null, null, "That row is gone; refresh the list", null],
    [null, null, null, { version: "1.0.0", url: "u" }],
  ])("anyLine_isShown", (line, warning, failed, newer) => {
    expect(showStatus(line, warning, failed, newer)).toBe(true);
  });
});

describe("listActions", () => {
  const recorder = () => {
    const reports: (string | null)[] = [];
    return { reports, act: listActions((e) => reports.push(e)) };
  };

  it("failure_reportsItsError", async () => {
    const { reports, act } = recorder();
    await act(Promise.reject("Set this workspace's tracker in settings"));
    expect(reports).toEqual([null, "Set this workspace's tracker in settings"]);
  });

  it("nextAction_clearsTheErrorAsItStarts_beforeItSettles", async () => {
    const { reports, act } = recorder();
    await act(Promise.reject("That row is gone; refresh the list"));
    let settle!: () => void;
    const running = act(new Promise<void>((resolve) => (settle = resolve)));
    expect(reports[reports.length - 1]).toBeNull();
    settle();
    await running;
    expect(reports).toEqual([null, "That row is gone; refresh the list", null]);
  });

  it("olderFailure_settlingAfterANewerAction_isNotShown", async () => {
    const { reports, act } = recorder();
    let fail!: (e: string) => void;
    const slow = act(new Promise<void>((_, reject) => (fail = reject)));
    await act(Promise.resolve());
    fail("That row is gone; refresh the list");
    await slow;
    expect(reports).toEqual([null, null]);
  });
});

describe("refreshTitle", () => {
  it("minutesAgo_readsAsASentence", () => {
    expect(refreshTitle(minutesAgo(3), 5, now)).toBe("Refresh now · checked 3 min ago · every 5 min");
  });

  it("underAMinute_isJustNow", () => {
    expect(refreshTitle(now.toISOString(), 1, now)).toBe("Refresh now · checked just now · every 1 min");
  });

  it("never_isNotCheckedYet", () => {
    expect(refreshTitle(null, 5, now)).toBe("Refresh now · not checked yet · every 5 min");
  });
});

describe("glows", () => {
  const prev = polled({ lastRefreshed: minutesAgo(5) });

  it("success_glows", () => {
    expect(glows(prev, polled())).toBe(true);
  });

  it("firstSnapshot_doesNotGlow", () => {
    expect(glows(null, polled())).toBe(false);
  });

  it("workspaceSwitch_doesNotGlow", () => {
    expect(glows(prev, polled({ workspace: "b" }))).toBe(false);
  });

  it("error_doesNotGlow", () => {
    expect(glows(prev, polled({ error: "Offline" }))).toBe(false);
  });

  it("unchangedLastRefreshed_doesNotGlow", () => {
    expect(glows(prev, polled({ lastRefreshed: prev.lastRefreshed }))).toBe(false);
  });

  it("pollingStarts_doesNotGlow", () => {
    expect(glows(prev, { ...prev, polling: true })).toBe(false);
  });
});

describe("newReviewsText", () => {
  it.each([[1, "1 new review"], [3, "3 new reviews"]])("%i_is_%s", (n, text) => {
    expect(newReviewsText(n)).toBe(text);
  });
});

describe("yawn", () => {
  it("freshWhileFocused_playsAtOnce", () => {
    expect(onFresh(0, 2, true)).toEqual({ held: 0, play: 2 });
  });

  it("freshWhileUnfocused_isHeld", () => {
    expect(onFresh(2, 3, false)).toEqual({ held: 5, play: 0 });
  });

  it("focus_playsTheHeldCount", () => {
    expect(onFocus(5)).toEqual({ held: 0, play: 5 });
  });

  it("focusWithNothingHeld_playsNothing", () => {
    expect(onFocus(0)).toEqual({ held: 0, play: 0 });
  });

  it("workspaceSwitch_dropsTheHeldCount", () => {
    expect(onSnapshot(5, "a", "b")).toBe(0);
  });

  it("sameWorkspace_keepsTheHeldCount", () => {
    expect(onSnapshot(5, "a", "a")).toBe(5);
  });

  it("firstSnapshot_keepsTheHeldCount", () => {
    expect(onSnapshot(5, null, "a")).toBe(5);
  });

  it("heldThenSwitchThenFocus_playsNothing", () => {
    const held = onSnapshot(onFresh(0, 3, false).held, "a", "b");
    expect(onFocus(held).play).toBe(0);
  });
});

describe("counterProblem", () => {
  const now = new Date("2026-11-18T12:00:00Z");
  const at = (minutes: number) => new Date(now.getTime() - minutes * 60_000).toISOString();
  const state = (over: Partial<{ hasPat: boolean; hasGithubToken: boolean; error: string | null; lastRefreshed: string | null }> = {}) =>
    ({ hasPat: true, hasGithubToken: false, error: null, lastRefreshed: at(1), ...over });

  it("flags no token at all", () => {
    expect(counterProblem(state({ hasPat: false }), 3, now)).toBe(true);
  });

  it("stays quiet on one failed poll while the data is recent", () => {
    expect(counterProblem(state({ error: "HTTP 503", lastRefreshed: at(4) }), 3, now)).toBe(false);
  });

  it("flags an error once the data is older than two polls, or never loaded", () => {
    expect(counterProblem(state({ error: "HTTP 503", lastRefreshed: at(8) }), 3, now)).toBe(true);
    expect(counterProblem(state({ error: "HTTP 503", lastRefreshed: null }), 3, now)).toBe(true);
  });

  it("stays quiet on stale data without an error", () => {
    expect(counterProblem(state({ lastRefreshed: at(60) }), 3, now)).toBe(false);
  });
});
