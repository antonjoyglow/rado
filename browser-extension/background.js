import { createGrouper, groupOpenTabs } from "./grouping.js";

const onTab = createGrouper(chrome);

// A tab opened from outside the browser has its target in pendingUrl at creation; catch it there
// and again once the URL commits, in case the first event came without it.
chrome.tabs.onCreated.addListener((tab) => onTab(tab, tab.pendingUrl || tab.url));
chrome.tabs.onUpdated.addListener((_tabId, change, tab) => {
  if (change.url) onTab(tab, change.url);
});
// Only when the browser or the extension starts, not on every wake of this worker: a tab the user dragged out of
// its group must stay out.
chrome.runtime.onStartup.addListener(() => groupOpenTabs(chrome, onTab));
chrome.runtime.onInstalled.addListener(() => groupOpenTabs(chrome, onTab));
