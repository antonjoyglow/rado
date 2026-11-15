import { describe, expect, it } from "vitest";
import type { CustomTheme } from "./api";
import { applyTheme, contrast, customPalette, parseHex, READABLE, unreadable } from "./theme";

const DARK: CustomTheme = { background: "#1f2125", text: "#e8eaee", accent: "#6d9ef6" };
const PAPER: CustomTheme = { background: "#fdf6e3", text: "#073642", accent: "#268bd2" };

describe("parseHex", () => {
  it("hexWithOrWithoutHash_isLowercaseWithHash", () => {
    expect([parseHex("#FFAA00"), parseHex(" ffaa00 "), parseHex("#0a0B0c")]).toEqual(["#ffaa00", "#ffaa00", "#0a0b0c"]);
  });

  it("anythingElse_isNull", () => {
    for (const typed of ["", "#fff", "#12345g", "red", "#1234567", "rgb(1,2,3)"]) expect(parseHex(typed), typed).toBeNull();
  });
});

describe("contrast", () => {
  it("blackOnWhite_is21AndOrderDoesNotMatter", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21);
    expect(contrast("#ffffff", "#000000")).toBeCloseTo(21);
    expect(contrast("#777777", "#777777")).toBeCloseTo(1);
  });
});

describe("customPalette", () => {
  it("setsEveryListVariable", () => {
    expect(Object.keys(customPalette(DARK)).sort()).toEqual(
      ["--accent", "--bg", "--clear", "--fg", "--hover", "--line", "--muted", "--normal", "--on-accent", "--urgent"],
    );
  });

  it("statusText_readsOnMidToneBackgrounds", () => {
    // Picks whose text itself reads; below that, unreadable() warns instead.
    for (const [background, text] of [["#808080", "#000000"], ["#3366cc", "#ffffff"], ["#b5309a", "#ffffff"], ["#2e7d32", "#ffffff"]]) {
      const p = customPalette({ background, text, accent: "#ffd60a" });
      for (const name of ["--muted", "--urgent", "--normal", "--clear"]) {
        expect(contrast(p[name], p["--bg"]), `${name} on ${background}`).toBeGreaterThanOrEqual(READABLE);
        expect(contrast(p[name], p["--hover"]), `${name} on hover of ${background}`).toBeGreaterThanOrEqual(READABLE);
      }
    }
  });

  it("keepsThePicksAsTheyAre", () => {
    const p = customPalette(PAPER);
    expect([p["--bg"], p["--fg"], p["--accent"]]).toEqual([PAPER.background, PAPER.text, PAPER.accent]);
  });

  it("mutedText_readsOnTheHoverColour", () => {
    for (const picks of [DARK, PAPER, { background: "#3a3a3a", text: "#c0c0c0", accent: "#ffcc00" }]) {
      const p = customPalette(picks);
      expect(contrast(p["--muted"], p["--hover"]), JSON.stringify(picks)).toBeGreaterThanOrEqual(READABLE);
    }
  });

  it("hover_tintsAwayFromTheTextWhenTowardItWouldNotRead", () => {
    const p = customPalette({ background: "#2e7d32", text: "#ffffff", accent: "#ffd60a" });
    expect(contrast(p["--fg"], p["--hover"])).toBeGreaterThanOrEqual(READABLE);
    // 7% of #e8eaee over #1f2125: lighter, toward the text.
    expect(customPalette(DARK)["--hover"]).toBe("#2d2f33");
  });

  it("textThatBarelyReads_isUsedForMutedToo", () => {
    const p = customPalette({ background: "#000000", text: "#767676", accent: "#ffffff" });
    expect(p["--muted"]).toBe("#767676");
  });

  it("onAccent_isBlackOrWhiteWhicheverReadsBetter", () => {
    expect(customPalette({ ...DARK, accent: "#ffd60a" })["--on-accent"]).toBe("#000000");
    expect(customPalette({ ...DARK, accent: "#2561c8" })["--on-accent"]).toBe("#ffffff");
  });

  it("statusColours_followTheBackgroundsLightness", () => {
    expect(customPalette(DARK)["--urgent"]).toBe("#f26b6b");
    expect(customPalette(PAPER)["--urgent"]).toBe("#c22d2d");
  });
});

describe("unreadable", () => {
  it("readablePicks_areNone", () => {
    expect(unreadable(DARK)).toEqual([]);
  });

  it("lowContrastPicks_areNamedWithTheirRatio", () => {
    expect(unreadable({ background: "#ffffff", text: "#cccccc", accent: "#eeeeee" })).toEqual(["Text 1.6:1", "Accent 1.2:1"]);
  });
});

describe("applyTheme", () => {
  const root = () => {
    const props = new Map<string, string>();
    const el = {
      dataset: {} as DOMStringMap,
      style: { setProperty: (k: string, v: string) => props.set(k, v), removeProperty: (k: string) => props.delete(k) },
    };
    return { el: el as unknown as HTMLElement, props };
  };

  it("custom_setsTheDerivedColoursInline", () => {
    const { el, props } = root();
    applyTheme(el, "custom", PAPER);
    expect(el.dataset.theme).toBe("custom");
    expect(Object.fromEntries(props)).toEqual(customPalette(PAPER));
  });

  it("presetAfterCustom_removesTheInlineColours", () => {
    const { el, props } = root();
    applyTheme(el, "custom", PAPER);
    applyTheme(el, "light", PAPER);
    expect(el.dataset.theme).toBe("light");
    expect(props.size).toBe(0);
  });
});
