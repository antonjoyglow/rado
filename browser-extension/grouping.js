// Rado opens every PR of an aggregate with "#rado=<branch>&group" on the URL; tabs carrying the
// same tag go into one tab group titled with the branch, reusing that group if it is already open.

const COLORS = ["blue", "green", "orange", "purple", "cyan", "pink", "yellow", "red"];

/** The branch in a URL's "#rado=<branch>&group" tag, or null when the URL is not part of an aggregate. */
export function groupTitle(url) {
  const match = /#rado=([^&#]+)&group(?:&|$)/.exec(url ?? "");
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}

/** A stable colour per branch, so a branch keeps its colour across reopenings. */
export function groupColor(title) {
  let hash = 0;
  for (const ch of title) hash = (hash * 31 + ch.codePointAt(0)) >>> 0;
  return COLORS[hash % COLORS.length];
}

/** Builds the tab handler; `chrome` is injected so tests can pass a fake. */
export function createGrouper(chrome) {
  const handled = new Set();
  // Rado opens several tabs at once; one at a time, so they find each other's group instead of racing.
  let queue = Promise.resolve();

  async function place(tab, title) {
    const [existing] = await chrome.tabGroups.query({ title, windowId: tab.windowId });
    if (existing) {
      await chrome.tabs.group({ groupId: existing.id, tabIds: [tab.id] });
      return;
    }
    const groupId = await chrome.tabs.group({ tabIds: [tab.id], createProperties: { windowId: tab.windowId } });
    await chrome.tabGroups.update(groupId, { title, color: groupColor(title) });
  }

  return function onTab(tab, url) {
    const title = groupTitle(url);
    if (!title || handled.has(tab.id)) return queue;
    handled.add(tab.id);
    queue = queue.then(() => place(tab, title)).catch((error) => console.warn("Rado tab group:", error));
    return queue;
  };
}
