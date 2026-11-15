import { describe, expect, it } from "vitest";
import type { Settings, Workspace } from "./api";
import { activeWorkspace, jiraSite, parseTeams, stepWorkspace, trackerFor, workspaceStep } from "./workspaces";

const ws = (id: string, site = ""): Workspace => ({ id, name: id.toUpperCase(), azureDevOps: { organization: "", teams: [] }, tracker: trackerFor(site) });
const list = [ws("a"), ws("b"), ws("c")];
const key = (k: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) =>
  ({ key: k, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });

describe("stepWorkspace", () => {
  it("moves to the neighbour", () => {
    expect(stepWorkspace(list, "b", 1)).toBe("c");
    expect(stepWorkspace(list, "b", -1)).toBe("a");
  });

  it("stops at either end instead of wrapping", () => {
    expect(stepWorkspace(list, "c", 1)).toBeNull();
    expect(stepWorkspace(list, "a", -1)).toBeNull();
  });

  it("does nothing for an unknown active id or a single workspace", () => {
    expect(stepWorkspace(list, "zz", 1)).toBeNull();
    expect(stepWorkspace([ws("a")], "a", 1)).toBeNull();
  });
});

describe("workspaceStep", () => {
  it("reads Ctrl+Left and Ctrl+Right", () => {
    expect(workspaceStep(key("ArrowLeft", { ctrlKey: true }))).toBe(-1);
    expect(workspaceStep(key("ArrowRight", { ctrlKey: true }))).toBe(1);
  });

  it("ignores plain arrows, other keys and extra modifiers", () => {
    expect(workspaceStep(key("ArrowRight"))).toBeNull();
    expect(workspaceStep(key("ArrowUp", { ctrlKey: true }))).toBeNull();
    expect(workspaceStep(key("ArrowRight", { ctrlKey: true, shiftKey: true }))).toBeNull();
    expect(workspaceStep(key("ArrowLeft", { ctrlKey: true, altKey: true }))).toBeNull();
  });
});

describe("trackers", () => {
  it("an empty site is no tracker, and a site is a Jira tracker", () => {
    expect(trackerFor("  ")).toEqual({ kind: "none" });
    expect(trackerFor(" acme.atlassian.net ")).toEqual({ kind: "jira", site: "acme.atlassian.net" });
  });

  it("jiraSite reads the active workspace's Jira site, or nothing", () => {
    const settings = { workspaces: [ws("a"), ws("b", "acme.atlassian.net")], activeWorkspace: "b" } as Settings;
    expect(jiraSite(activeWorkspace(settings))).toBe("acme.atlassian.net");
    expect(jiraSite(activeWorkspace({ ...settings, activeWorkspace: "a" }))).toBe("");
    expect(jiraSite(activeWorkspace({ ...settings, activeWorkspace: "gone" }))).toBe("");
  });
});

describe("parseTeams", () => {
  it("splits on commas and drops blanks", () => {
    expect(parseTeams(" TeamAlpha, ,Beta ,")).toEqual(["TeamAlpha", "Beta"]);
  });
});
