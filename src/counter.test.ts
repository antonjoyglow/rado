import { describe, expect, it } from "vitest";
import { COUNTER_DEFAULT, counterVars, HOVER_RANGE, numberColour, REST_RANGE } from "./counter";
import { heat, parseWhole } from "./format";

describe("numberColour", () => {
  it("uses heat for the count when no colour is picked", () => {
    expect(numberColour(null, 0)).toBe(heat(0));
    expect(numberColour(null, 12)).toBe(heat(12));
  });

  it("uses the picked colour whatever the count", () => {
    expect(numberColour("#ffd60a", 0)).toBe("#ffd60a");
    expect(numberColour("#ffd60a", 30)).toBe("#ffd60a");
  });
});

describe("typed opacity", () => {
  it.each([
    ["55", REST_RANGE, 55],
    [" 20 ", REST_RANGE, 20],
    ["100", REST_RANGE, 100],
    ["0", HOVER_RANGE, 0],
    ["8", HOVER_RANGE, 8],
  ] as const)("%j in %j -> %i", (text, range, value) => expect(parseWhole(text, range)).toBe(value));

  it.each([
    ["19", REST_RANGE],
    ["101", HOVER_RANGE],
    ["-1", HOVER_RANGE],
    ["55.5", REST_RANGE],
    ["", HOVER_RANGE],
    ["half", REST_RANGE],
  ] as const)("rejects %j in %j", (text, range) => expect(parseWhole(text, range)).toBeNull());
});

describe("counterVars", () => {
  it("gives the original look for the defaults at 100%", () => {
    expect(counterVars(COUNTER_DEFAULT, 100)).toEqual({
      "--pill-scale": "1",
      "--pill-logo": "#ffffff",
      "--pill-rest": "0.55",
      "--pill-hover": "0.08",
    });
  });

  it("turns percents into fractions", () => {
    const vars = counterVars({ ...COUNTER_DEFAULT, logo: "#c792ea", restOpacity: 100, hoverOpacity: 0 }, 150);
    expect(vars).toEqual({ "--pill-scale": "1.5", "--pill-logo": "#c792ea", "--pill-rest": "1", "--pill-hover": "0" });
  });
});
