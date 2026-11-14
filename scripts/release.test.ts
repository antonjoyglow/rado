import { describe, expect, it } from "vitest";
// @ts-expect-error plain .mjs script without type declarations
import { buildSection, bumpVersions, parseTitle, sectionBody, upsertSection } from "./release.mjs";

describe("parseTitle", () => {
  it("featWithScope_dropsTheReleaseSuffixAndKeepsThePrNumber", () => {
    expect(parseTitle("feat(list): header tabs for the review list and settings, release 0.7.0 (#8)"))
      .toEqual({ type: "feat", text: "list: header tabs for the review list and settings (#8)" });
  });

  it("fixWithoutScope_keepsTheText", () => {
    expect(parseTitle("fix: scroll the settings panel, release 0.4.2 (#2)")).toEqual({ type: "fix", text: "scroll the settings panel (#2)" });
  });

  it("breakingMarker_isAccepted", () => {
    expect(parseTitle("feat(api)!: drop v1 (#9)")).toEqual({ type: "feat", text: "api: drop v1 (#9)" });
  });

  it("otherTypesAndFreeText_areDropped", () => {
    expect(parseTitle("chore: update dependencies, release 0.4.1 (#1)")).toBeNull();
    expect(parseTitle("ci: add PR checks (#9)")).toBeNull();
    expect(parseTitle("Rado 0.4.0")).toBeNull();
    expect(parseTitle("feature: not conventional")).toBeNull();
  });
});

describe("buildSection", () => {
  it("groupsFeaturesThenFixes_inCommitOrder", () => {
    const section = buildSection("0.8.0", "2026-11-20", [
      "feat(list): b (#11)",
      "chore: deps (#10)",
      "fix(tray): c (#9)",
      "feat: a (#8)",
    ]);
    expect(section).toBe("## [0.8.0] - 2026-11-20\n\n### Features\n\n- list: b (#11)\n- a (#8)\n\n### Fixes\n\n- tray: c (#9)\n");
  });

  it("noFeatOrFix_isJustTheHeading", () => {
    expect(buildSection("0.8.0", "2026-11-20", ["chore: deps"])).toBe("## [0.8.0] - 2026-11-20\n");
  });
});

const changelog = "# Changelog\n\nIntro.\n\n## [0.7.0] - 2026-11-14\n\n### Features\n\n- old (#8)\n";

describe("upsertSection", () => {
  it("newVersion_goesAboveTheNewestSection", () => {
    const out = upsertSection(changelog, "0.8.0", "## [0.8.0] - 2026-11-20\n\n### Fixes\n\n- x (#9)\n");
    expect(out).toBe("# Changelog\n\nIntro.\n\n## [0.8.0] - 2026-11-20\n\n### Fixes\n\n- x (#9)\n\n## [0.7.0] - 2026-11-14\n\n### Features\n\n- old (#8)\n");
  });

  it("rerun_replacesThatVersionInsteadOfDuplicatingIt", () => {
    const once = upsertSection(changelog, "0.8.0", "## [0.8.0] - 2026-11-20\n\n### Fixes\n\n- x (#9)\n");
    const twice = upsertSection(once, "0.8.0", "## [0.8.0] - 2026-11-21\n\n### Fixes\n\n- x (#9)\n- y (#10)\n");
    expect(twice.match(/## \[0\.8\.0\]/g)).toHaveLength(1);
    expect(twice).toContain("- y (#10)\n\n## [0.7.0]");
    expect(upsertSection(twice, "0.8.0", "## [0.8.0] - 2026-11-21\n\n### Fixes\n\n- x (#9)\n- y (#10)\n")).toBe(twice);
  });

  it("lastSection_isReplacedToTheEnd", () => {
    const out = upsertSection(changelog, "0.7.0", "## [0.7.0] - 2026-11-14\n\n### Features\n\n- new (#8)\n");
    expect(out).toBe("# Changelog\n\nIntro.\n\n## [0.7.0] - 2026-11-14\n\n### Features\n\n- new (#8)\n\n");
  });

  it("crlfInput_isHandled", () => {
    const out = upsertSection(changelog.replace(/\n/g, "\r\n"), "0.7.0", "## [0.7.0] - 2026-11-14\n\n- new\n");
    expect(out.match(/## \[0\.7\.0\]/g)).toHaveLength(1);
  });
});

describe("sectionBody", () => {
  const two = upsertSection(changelog, "0.8.0", "## [0.8.0] - 2026-11-20\n\n### Fixes\n\n- x (#9)\n");

  it("returnsTheBodyWithoutTheHeadingOrTheNextSection", () => {
    expect(sectionBody(two, "0.8.0")).toBe("### Fixes\n\n- x (#9)");
    expect(sectionBody(two.replace(/\n/g, "\r\n"), "0.7.0")).toBe("### Features\n\n- old (#8)");
  });

  it("missingOrEmptySection_isNull", () => {
    expect(sectionBody(two, "0.9.0")).toBeNull();
    expect(sectionBody("## [0.9.0] - 2026-12-01\n\n## [0.8.0]\n\n- x\n", "0.9.0")).toBeNull();
  });

  it("headingOnTheLastLineWithoutNewline_isNull", () => {
    expect(sectionBody("# Changelog\n\nIntro.\n\n## [0.8.0] - d", "0.8.0")).toBeNull();
  });

  it("versionDots_areLiteral", () => {
    expect(sectionBody("## [0x8x0] - d\n\n- wrong\n", "0.8.0")).toBeNull();
  });
});

describe("bumpVersions", () => {
  const files = {
    "package.json": '{\n  "name": "rado",\n  "version": "0.7.0",\n  "dependencies": {\n    "vite": "^8"\n  }\n}\n',
    "package-lock.json": '{\r\n  "name": "rado",\r\n  "version": "0.7.0",\r\n  "packages": {\r\n    "": {\r\n      "version": "0.7.0"\r\n    },\r\n    "node_modules/x": {\r\n      "version": "0.7.0"\r\n    }\r\n  }\r\n}\r\n',
    "src-tauri/tauri.conf.json": '{\n  "productName": "Rado",\n  "version": "0.7.0",\n  "plugins": { "x": { "version": "1" } }\n}\n',
    "src-tauri/Cargo.toml": '[package]\nname = "rado"\nversion = "0.7.0"\n\n[dependencies]\ntauri = { version = "2" }\n',
    "src-tauri/Cargo.lock": '[[package]]\r\nname = "quote"\r\nversion = "0.7.0"\r\n\r\n[[package]]\r\nname = "rado"\r\nversion = "0.7.0"\r\n',
  };
  const out = bumpVersions(files, "0.8.0");

  it("setsOnlyRadosOwnVersion", () => {
    expect(JSON.parse(out["package.json"]).version).toBe("0.8.0");
    const lock = JSON.parse(out["package-lock.json"]);
    expect([lock.version, lock.packages[""].version, lock.packages["node_modules/x"].version]).toEqual(["0.8.0", "0.8.0", "0.7.0"]);
    expect(out["src-tauri/tauri.conf.json"]).toBe(files["src-tauri/tauri.conf.json"].replace('"0.7.0"', '"0.8.0"'));
    expect(out["src-tauri/Cargo.toml"]).toBe(files["src-tauri/Cargo.toml"].replace('"0.7.0"', '"0.8.0"'));
    expect(out["src-tauri/Cargo.lock"]).toBe(files["src-tauri/Cargo.lock"].replace('name = "rado"\r\nversion = "0.7.0"', 'name = "rado"\r\nversion = "0.8.0"'));
  });

  it("keepsJsonFormattingAndLineEndings", () => {
    expect(out["package.json"]).toBe(files["package.json"].replace("0.7.0", "0.8.0"));
    expect(out["package-lock.json"]).toBe(files["package-lock.json"].replace(/0\.7\.0/, "0.8.0").replace('"": {\r\n      "version": "0.7.0"', '"": {\r\n      "version": "0.8.0"'));
  });

  it("missingVersion_throws", () => {
    expect(() => bumpVersions({ ...files, "src-tauri/Cargo.lock": 'name = "other"\n' }, "0.8.0")).toThrow(/Cargo\.lock/);
  });
});
