import { describe, expect, it } from "vitest";
import { extensionMessage } from "./extension";

const folder = "C:\\Users\\me\\AppData\\Local\\Rado\\browser-extension";

describe("extensionMessage", () => {
  it("loadedFromTheBundledFolder_needsNoSteps", () => {
    expect(extensionMessage({ folder, state: "loaded", from: null }, "edge")).toEqual({ text: "Tab groups extension: loaded", steps: null });
  });

  it("loadedFromAnotherFolder_namesIt", () => {
    expect(extensionMessage({ folder, state: "loaded", from: "C:\\dev\\rado\\browser-extension" }, "edge").text).toBe("Tab groups extension: loaded from C:\\dev\\rado\\browser-extension");
  });

  it("notLoaded_givesTheLoadUnpackedStepsForThatBrowser", () => {
    expect(extensionMessage({ folder, state: "notLoaded" }, "chrome").steps).toBe(
      `Set up copies chrome://extensions: paste it into the new tab and press Enter. Switch on Developer mode, click Load unpacked and pick ${folder} (selected in Explorer).`,
    );
  });

  it("gone_saysToRemoveTheOldCopyFirst", () => {
    const m = extensionMessage({ folder, state: "gone", path: "C:\\dev\\rado\\edge-extension" }, "edge");
    expect(m.text).toBe("Tab groups extension: loaded from C:\\dev\\rado\\edge-extension, which no longer exists");
    expect(m.steps).toContain("edge://extensions: paste it into the new tab and press Enter. Remove that copy. Switch on Developer mode");
  });

  it("off_saysToSwitchItOn", () => {
    expect(extensionMessage({ folder, state: "off" }, "edge").steps).toMatch(/Switch it on there\.$/);
  });
});
