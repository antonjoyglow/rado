import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** "stale": a review with no activity for three weeks, left out of the count. */
export type ReviewState = "needsReview" | "reReview" | "waitingOnAuthor" | "approved" | "mine" | "stale";
export type Tier = "low" | "normal" | "urgent";
export type Provider = "ado" | "github";

/** Which PR, as people write it: provider plus the number shown as `!n` (Azure DevOps) or `#n` (GitHub). */
export interface PrRef {
  repo: string;
  provider: Provider;
  number: number;
}

export interface Member extends PrRef {
  /** Unique across sources; what openPr takes. */
  id: number;
  title: string;
  url: string;
  /** null for a PR on the same branch that you are not asked to review. */
  state: ReviewState | null;
  createdAt: string;
  /** Last thread activity after opening; null when there was none, its threads failed to load, or it isn't assigned to you. */
  updatedAt: string | null;
}

/** A list row: one PR, or every PR sharing a source branch, summarised. */
export interface ReviewItem extends PrRef {
  key: string;
  id: number;
  title: string;
  branch: string;
  members: Member[];
  /** Age of the row's most recently opened PR. */
  newestAgeHours: number;
  /** The tracker's issue key in the branch name, if any; none without a tracker. */
  ticket: string | null;
  author: string;
  target: string;
  /** Merges into a release branch (master, main, qa, release); the list names the target only then. */
  releaseTarget: boolean;
  ageHours: number;
  /** When the row's earliest PR assigned to you (or yours) was opened. */
  createdAt: string;
  /** Latest activity across those PRs, a later sibling's opening included; null when there was none or it is unknown. */
  updatedAt: string | null;
  required: boolean;
  draft: boolean;
  state: ReviewState;
  score: number;
  tier: Tier;
  hidden: boolean;
  approvals: number;
  objections: number;
  comments: { total: number; openThreads: number } | null;
  /** Waiting on you: a review to do, or your own PR with rejections or unresolved threads. Never true for a hidden row. */
  actionable: boolean;
}

/**
 * Where ticket keys in branch names link to: a Jira host, or a Linear workspace slug (`acme` in linear.app/acme).
 * `keys` are the team or project keys (`HOO`); when set, only branch keys with one of them count.
 */
export type Tracker =
  | { kind: "none" }
  | { kind: "jira"; site: string; keys: string[] }
  | { kind: "linear"; workspace: string; keys: string[] };

/** A named set of sources; only the active one is polled. A source is on once its secret is saved. */
export interface Workspace {
  /** Stable across renames; names the workspace's keyring entries and data folder. */
  id: string;
  name: string;
  azureDevOps: { organization: string; teams: string[] };
  /** `repos` (`owner/name`): every open PR there, other than drafts and mine, counts as waiting on my review. */
  github: { repos: string[] };
  tracker: Tracker;
}

/** Which secrets a workspace has saved. */
export interface Saved {
  pat: boolean;
  githubToken: boolean;
}

export interface Settings {
  pollMinutes: number;
  notify: boolean;
  /** Counter size in percent, 75–200; set in the Counter tab. */
  pillScale: number;
  /** Text size of the review list in percent, 80–150. */
  textScale: number;
  /** Size of the review list in percent of its screen: width 20–100 (never under 360 px), height 25–100. */
  listWidthPercent: number;
  listHeightPercent: number;
  /** Where links open: the system's default browser, Edge or Chrome. */
  browser: Browser;
  /** Colours of the review list; the counter has its own, in `counter`. */
  theme: Theme;
  customTheme: CustomTheme;
  counter: CounterStyle;
  workspaces: Workspace[];
  activeWorkspace: string;
}

export type Browser = "system" | "edge" | "chrome";
/** The tab groups extension in Edge or Chrome. `folder` is where the installer put it; `from` is set when the browser loads it from elsewhere. */
export type ExtensionStatus = { folder: string } & (
  | { state: "loaded"; from: string | null }
  | { state: "gone"; path: string }
  | { state: "off" | "notLoaded" }
);
/** "system" follows Windows' light or dark app mode. */
export type Theme = "system" | "dark" | "light" | "highContrast" | "custom";
/** The three colours a custom theme is picked from, as `#rrggbb`; the list derives the rest. */
export interface CustomTheme {
  background: string;
  text: string;
  accent: string;
}

/** The counter's look. A number colour of null is heat: blue at 0 through to red at 10 or more. */
export interface CounterStyle {
  reviews: string | null;
  mine: string | null;
  /** The bird between the numbers, and the `!`, as `#rrggbb`. */
  logo: string;
  /** Opacity in percent: 20–100 at rest, 0–100 near the cursor. */
  restOpacity: number;
  hoverOpacity: number;
}

export interface Snapshot {
  items: ReviewItem[];
  /** The counter's numbers, in rows: waiting on your review, and your own rows that need work. */
  counts: { reviews: number; mine: number };
  lastRefreshed: string | null;
  polling: boolean;
  error: string | null;
  hasPat: boolean;
  hasGithubToken: boolean;
  /** What went wrong loading the settings, shown for the whole run. */
  warning: string | null;
  settings: Settings;
  /** By workspace id. */
  saved: Record<string, Saved>;
  /** A GitHub release newer than this Rado; its link opens the installer in the browser. */
  newerRelease: { version: string; url: string } | null;
}

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  refresh: () => invoke<void>("refresh_now"),
  openList: () => invoke<void>("open_list"),
  hideList: () => invoke<void>("hide_list"),
  openPr: (id: number) => invoke<void>("open_pr", { id }),
  openRow: (key: string) => invoke<void>("open_row", { key }),
  openTicket: (key: string) => invoke<void>("open_ticket", { key }),
  copyRow: (key: string) => invoke<void>("copy_row", { key }),
  setHidden: (key: string, hidden: boolean) => invoke<void>("set_hidden", { key, hidden }),
  savePat: (workspace: string, pat: string) => invoke<void>("save_pat", { workspace, pat }),
  saveGithubToken: (workspace: string, token: string) => invoke<void>("save_github_token", { workspace, token }),
  removePat: (workspace: string) => invoke<void>("remove_pat", { workspace }),
  removeGithubToken: (workspace: string) => invoke<void>("remove_github_token", { workspace }),
  /** The general settings; the workspaces in it are ignored. */
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  addWorkspace: (name: string) => invoke<string>("add_workspace", { name }),
  saveWorkspace: (workspace: Workspace) => invoke<void>("save_workspace", { workspace }),
  switchWorkspace: (id: string) => invoke<void>("switch_workspace", { id }),
  deleteWorkspace: (id: string) => invoke<void>("delete_workspace", { id }),
  autostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  openNewerRelease: () => invoke<void>("open_newer_release"),
  extensionStatus: (browser: "edge" | "chrome") => invoke<ExtensionStatus>("extension_status", { browser }),
  setUpExtension: (browser: "edge" | "chrome") => invoke<void>("set_up_extension", { browser }),
};

export async function watchSnapshot(render: (s: Snapshot) => void): Promise<void> {
  await listen<Snapshot>("snapshot", (e) => render(e.payload));
  render(await api.snapshot());
}

/** Once per refresh that brought PRs newly waiting on your review, with how many. */
export async function watchFresh(on: (count: number) => void): Promise<void> {
  await listen<number>("fresh", (e) => on(e.payload));
}
