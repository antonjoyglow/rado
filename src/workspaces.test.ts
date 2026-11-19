import { describe, expect, it, vi } from "vitest";
import { api, type Settings, type Workspace } from "./api";
import { DROPPED } from "./save";
import { activeWorkspace, parseList, stepWorkspace, trackerFor, trackerKeys, trackerName, trackerValue, workspaceSaver, workspaceStep } from "./workspaces";

vi.mock("./api", () => ({ api: { saveWorkspace: vi.fn() } }));

const ws = (id: string, tracker: Workspace["tracker"] = { kind: "none" }): Workspace => ({ id, name: id.toUpperCase(), azureDevOps: { organization: "", teams: [] }, github: { repos: [] }, tracker });
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
    expect(trackerFor("jira", "  ", ["HOO"])).toEqual({ kind: "none" });
    expect(trackerFor("none", "acme", [])).toEqual({ kind: "none" });
    expect(trackerFor("jira", " acme.atlassian.net ", [])).toEqual({ kind: "jira", site: "acme.atlassian.net", keys: [] });
    expect(trackerFor("linear", " acme ", ["HOO"])).toEqual({ kind: "linear", workspace: "acme", keys: ["HOO"] });
  });

  it("trackerKeys reads back the team keys, none without a tracker", () => {
    expect(trackerKeys({ kind: "linear", workspace: "acme", keys: ["HOO"] })).toEqual(["HOO"]);
    expect(trackerKeys({ kind: "none" })).toEqual([]);
  });

  it("trackerValue reads back the field of each kind", () => {
    expect(trackerValue({ kind: "jira", site: "acme.atlassian.net", keys: [] })).toBe("acme.atlassian.net");
    expect(trackerValue({ kind: "linear", workspace: "acme", keys: [] })).toBe("acme");
    expect(trackerValue({ kind: "none" })).toBe("");
  });

  it("trackerName names the active workspace's tracker, or nothing", () => {
    const settings = {
      workspaces: [ws("a"), ws("b", { kind: "jira", site: "acme.atlassian.net", keys: [] }), ws("c", { kind: "linear", workspace: "acme", keys: [] })],
      activeWorkspace: "b",
    } as Settings;
    expect(trackerName(activeWorkspace(settings))).toBe("Jira");
    expect(trackerName(activeWorkspace({ ...settings, activeWorkspace: "c" }))).toBe("Linear");
    expect(trackerName(activeWorkspace({ ...settings, activeWorkspace: "a" }))).toBe("");
    expect(trackerName(activeWorkspace({ ...settings, activeWorkspace: "gone" }))).toBe("");
  });
});

describe("workspaceSaver", () => {
  it("a failed save puts the workspace back to the last saved one and drops the saves queued behind it", async () => {
    const send = vi.mocked(api.saveWorkspace);
    send.mockReset();
    send.mockResolvedValueOnce().mockRejectedValueOnce("Not saved: “x” isn't a GitHub repo (owner/name)").mockResolvedValue();
    const saver = workspaceSaver(ws("r"));
    let reverted: Workspace | null = null;
    saver.revert = (last) => (reverted = last);

    await saver.save({ ...ws("r"), name: "Renamed" });
    const bad = saver.save({ ...ws("r"), name: "Renamed", github: { repos: ["x"] } });
    const queued = saver.save({ ...ws("r"), name: "Queued", github: { repos: ["x"] } });

    await expect(bad).rejects.toMatch(/^Not saved/);
    await expect(queued).rejects.toBe(DROPPED);
    expect(reverted).toEqual({ ...ws("r"), name: "Renamed" });
    expect(saver.latest).toEqual({ ...ws("r"), name: "Renamed" });
    expect(send).toHaveBeenCalledTimes(2);
  });
});

describe("parseList", () => {
  it("splits on commas and drops blanks", () => {
    expect(parseList(" TeamAlpha, ,Beta ,")).toEqual(["TeamAlpha", "Beta"]);
  });
});
