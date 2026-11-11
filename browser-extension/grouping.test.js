import { describe, expect, it } from "vitest";
import { createGrouper, groupColor, groupTitle } from "./grouping.js";

function fakeChrome() {
  const groups = [];
  const membership = new Map();
  let nextId = 100;
  const delay = () => new Promise((resolve) => setTimeout(resolve, 1));
  return {
    groups,
    membership,
    tabGroups: {
      async query({ title, windowId }) {
        await delay();
        return groups.filter((g) => g.title === title && g.windowId === windowId);
      },
      async update(id, props) {
        await delay();
        Object.assign(groups.find((g) => g.id === id), props);
      },
    },
    tabs: {
      async group({ groupId, tabIds, createProperties }) {
        await delay();
        const id = groupId ?? nextId++;
        if (groupId === undefined) groups.push({ id, windowId: createProperties.windowId, title: "" });
        for (const tabId of tabIds) membership.set(tabId, id);
        return id;
      },
    },
  };
}

describe("groupTitle", () => {
  it.each([
    ["https://dev.azure.com/o/p/_git/r/pullrequest/1#rado=ABC-4827&group", "ABC-4827"],
    ["https://x/1#rado=feature%2Fsearch-fix&group", "feature/search-fix"],
    ["https://x/1", null],
    ["https://x/1#rado=ABC-4827", null],
    ["https://x/1#rado=ABC-4827&grouping", null],
    ["https://x/1#other=1", null],
    [undefined, null],
    ["https://x/1#rado=%E0%A4%A&group", null],
  ])("%s -> %s", (url, title) => expect(groupTitle(url)).toBe(title));
});

describe("groupColor", () => {
  it("sameTitle_sameColor", () => expect(groupColor("ABC-4827")).toBe(groupColor("ABC-4827")));
});

describe("createGrouper", () => {
  it("tabsOpenedTogether_landInOneTitledGroup", async () => {
    const chrome = fakeChrome();
    const onTab = createGrouper(chrome);
    onTab({ id: 1, windowId: 7 }, "https://x/1#rado=ABC-4827&group");
    onTab({ id: 2, windowId: 7 }, "https://x/2#rado=ABC-4827&group");
    await onTab({ id: 3, windowId: 7 }, "https://x/3#rado=ABC-4827&group");
    expect(chrome.groups).toHaveLength(1);
    expect(chrome.groups[0]).toMatchObject({ title: "ABC-4827", color: groupColor("ABC-4827") });
    expect([...chrome.membership.values()]).toEqual([100, 100, 100]);
  });

  it("differentBranches_getSeparateGroups", async () => {
    const chrome = fakeChrome();
    const onTab = createGrouper(chrome);
    onTab({ id: 1, windowId: 7 }, "https://x/1#rado=a&group");
    await onTab({ id: 2, windowId: 7 }, "https://x/2#rado=b&group");
    expect(chrome.groups.map((g) => g.title)).toEqual(["a", "b"]);
  });

  it("untaggedOrRepeatedTab_isLeftAlone", async () => {
    const chrome = fakeChrome();
    const onTab = createGrouper(chrome);
    await onTab({ id: 1, windowId: 7 }, "https://x/1");
    await onTab({ id: 2, windowId: 7 }, "https://x/2#rado=a&group");
    await onTab({ id: 2, windowId: 7 }, "https://x/2#rado=a&group");
    expect(chrome.groups).toHaveLength(1);
    expect(chrome.membership.size).toBe(1);
  });

  it("groupInAnotherWindow_isNotReused", async () => {
    const chrome = fakeChrome();
    const onTab = createGrouper(chrome);
    onTab({ id: 1, windowId: 7 }, "https://x/1#rado=a&group");
    await onTab({ id: 2, windowId: 8 }, "https://x/2#rado=a&group");
    expect(chrome.groups).toHaveLength(2);
  });
});
