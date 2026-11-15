import { describe, expect, it } from "vitest";
import type { Settings, Workspace } from "./api";
import { activeWorkspace, parseTeams, stepWorkspace, trackerFor, trackerName, trackerValue, workspaceStep } from "./workspaces";

const ws = (id: string, tracker: Workspace["tracker"] = { kind: "none" }): Workspace => ({ id, name: id.toUpperCase(), azureDevOps: { organization: "", teams: [] }, tracker });
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
  it("an empty field or None is no tracker; otherwise the field goes to the chosen kind", () => {
    expect(trackerFor("jira", "  ")).toEqual({ kind: "none" });
    expect(trackerFor("none", "acme")).toEqual({ kind: "none" });
    expect(trackerFor("jira", " acme.atlassian.net ")).toEqual({ kind: "jira", site: "acme.atlassian.net" });
    expect(trackerFor("linear", " acme ")).toEqual({ kind: "linear", workspace: "acme" });
  });

  it("trackerValue reads back the field of each kind", () => {
    expect(trackerValue({ kind: "jira", site: "acme.atlassian.net" })).toBe("acme.atlassian.net");
    expect(trackerValue({ kind: "linear", workspace: "acme" })).toBe("acme");
    expect(trackerValue({ kind: "none" })).toBe("");
  });

  it("trackerName names the active workspace's tracker, or nothing", () => {
    const settings = {
      workspaces: [ws("a"), ws("b", { kind: "jira", site: "acme.atlassian.net" }), ws("c", { kind: "linear", workspace: "acme" })],
      activeWorkspace: "b",
    } as Settings;
    expect(trackerName(activeWorkspace(settings))).toBe("Jira");
    expect(trackerName(activeWorkspace({ ...settings, activeWorkspace: "c" }))).toBe("Linear");
    expect(trackerName(activeWorkspace({ ...settings, activeWorkspace: "a" }))).toBe("");
    expect(trackerName(activeWorkspace({ ...settings, activeWorkspace: "gone" }))).toBe("");
  });
});

describe("parseTeams", () => {
  it("splits on commas and drops blanks", () => {
    expect(parseTeams(" TeamAlpha, ,Beta ,")).toEqual(["TeamAlpha", "Beta"]);
  });
});
