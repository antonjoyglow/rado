import { createGrouper, groupOpenTabs } from "./grouping.js";

const onTab = createGrouper(chrome);

// A tab opened from outside the browser has its target in pendingUrl at creation; catch it there
// and again once the URL commits, in case the first event came without it.
chrome.tabs.onCreated.addListener((tab) => onTab(tab, tab.pendingUrl || tab.url));
chrome.tabs.onUpdated.addListener((_tabId, change, tab) => {
  if (change.url) onTab(tab, change.url);
});
// From title.js, which sees the tag before Azure DevOps drops it from the URL.
chrome.runtime.onMessage.addListener((message, sender) => {
  if (sender.tab && typeof message?.radoUrl === "string") onTab(sender.tab, message.radoUrl);
});
// Only when the browser starts and on a first install, not on every wake of this worker or on a reload or update of
// the extension: a tab the user dragged out of its group must stay out.
chrome.runtime.onStartup.addListener(() => groupOpenTabs(chrome, onTab));
chrome.runtime.onInstalled.addListener(({ reason }) => {
  if (reason === "install") groupOpenTabs(chrome, onTab);
});
