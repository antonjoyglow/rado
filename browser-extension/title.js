// Content script: a PR tab that Rado opened ("#rado=<branch>" on the URL) is titled "repo:branch".
// Azure DevOps and GitHub rewrite document.title as they render, so the title is reasserted on every change.
// Runs at document_start, before the page's own scripts can drop the URL fragment.

/** "repo:branch" for a Rado-tagged Azure DevOps or GitHub PR URL, or null. */
function radoTabTitle(pathname, hash) {
  const repo = (/\/_git\/([^/]+)\/pullrequest\/\d+/.exec(pathname) ?? /^\/[^/]+\/([^/]+)\/pull\/\d+/.exec(pathname))?.[1];
  const branch = /^#rado=([^&#]+)/.exec(hash)?.[1];
  if (!repo || !branch) return null;
  try {
    return `${decodeURIComponent(repo)}:${decodeURIComponent(branch)}`;
  } catch {
    return null;
  }
}

/** The PR page a stored title belongs to, so navigating elsewhere in the tab restores the site's own title. */
function prPath(pathname) {
  return /^.*\/pullrequest\/\d+/.exec(pathname)?.[0] ?? /^\/[^/]+\/[^/]+\/pull\/\d+/.exec(pathname)?.[0] ?? null;
}

function main() {
  const KEY = "rado-tab-title";
  const fresh = radoTabTitle(location.pathname, location.hash);
  if (fresh) sessionStorage.setItem(KEY, JSON.stringify({ path: prPath(location.pathname), title: fresh }));
  // sessionStorage keeps the title across reloads, after ADO has dropped the fragment.
  const saved = JSON.parse(sessionStorage.getItem(KEY) ?? "null");
  if (!saved) return;

  // The observer fires on every DOM change, so this check stays cheap.
  const wanted = () => (prPath(location.pathname) === saved.path ? saved.title : null);
  const apply = () => {
    const title = wanted();
    if (title && document.title !== title) document.title = title;
  };

  new MutationObserver(apply).observe(document, { subtree: true, childList: true, characterData: true });
  apply();
}

if (typeof chrome !== "undefined" && chrome.runtime?.id) main();
