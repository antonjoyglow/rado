// Snapshot fixtures shaped like the types in src/bindings; update them when those change.
const iso = (hoursAgo) => new Date(Date.now() - hoursAgo * 3600_000).toISOString();

export function workspace(id, name, extra = {}) {
  return {
    id,
    name,
    azureDevOps: { organization: "contoso", teams: [] },
    github: { repos: [] },
    tracker: { kind: "none" },
    ...extra,
  };
}

export function settings(extra = {}) {
  const workspaces = extra.workspaces ?? [workspace("ws1", "Work")];
  return {
    pollMinutes: 3,
    notify: true,
    pillScale: 100,
    textScale: 100,
    listWidthPercent: 33,
    listHeightPercent: 50,
    browser: "system",
    theme: "dark",
    customTheme: { background: "#1f2125", text: "#e8eaee", accent: "#6d9ef6" },
    counter: { reviews: null, mine: null, logo: "#ffffff", restOpacity: 55, hoverOpacity: 8 },
    activeWorkspace: workspaces[0].id,
    ...extra,
    workspaces,
  };
}

let nextId = 1000;
export function member(given = {}) {
  const extra = Object.fromEntries(Object.entries(given).filter(([, v]) => v !== undefined));
  const id = extra.id ?? nextId++;
  return {
    id,
    provider: "azureDevOps",
    number: id,
    repo: "web",
    title: `PR ${id}`,
    url: `https://dev.azure.com/contoso/p/_git/web/pullrequest/${id}`,
    state: "needsReview",
    createdAt: iso(5),
    updatedAt: iso(1),
    agentDraft: null,
    ...extra,
  };
}

export function item(given = {}) {
  const extra = Object.fromEntries(Object.entries(given).filter(([, v]) => v !== undefined));
  const members = extra.members ?? [member({ id: extra.id, title: extra.title, state: extra.state ?? "needsReview", provider: extra.provider })];
  const m = members[0];
  return {
    key: extra.key ?? `${m.provider}:${m.id}`,
    members,
    newestAgeHours: 5,
    ticket: null,
    id: m.id,
    provider: m.provider,
    number: m.number,
    title: m.title,
    repo: m.repo,
    branch: "feature/x",
    author: "Ada Lovelace",
    target: "main",
    releaseTarget: false,
    ageHours: 5,
    createdAt: iso(5),
    updatedAt: iso(1),
    required: true,
    viaGroup: false,
    draft: false,
    state: "needsReview",
    score: 1,
    tier: "normal",
    hidden: false,
    approvals: 0,
    objections: 0,
    comments: null,
    actionable: true,
    ...extra,
  };
}

export function snapshot(extra = {}) {
  const s = settings(extra.settings);
  const items = extra.items ?? [item()];
  return {
    warning: null,
    newerRelease: null,
    items,
    counts: { reviews: items.filter((i) => i.actionable && i.state !== "mine").length, mine: items.filter((i) => i.state === "mine" && i.actionable).length },
    lastRefreshed: iso(0.01),
    polling: false,
    error: null,
    hasPat: true,
    hasGithubToken: false,
    saved: Object.fromEntries(s.workspaces.map((w) => [w.id, { pat: true, githubToken: false }])),
    ...extra,
    settings: s,
  };
}
