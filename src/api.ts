import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ExtensionStatus } from "./bindings/ExtensionStatus";
import type { Member } from "./bindings/Member";
import type { Settings } from "./bindings/Settings";
import type { Snapshot } from "./bindings/Snapshot";
import type { Workspace } from "./bindings/Workspace";

// Generated from the Rust types by `cargo test` (ts-rs) into ./bindings; change them in Rust.
export type { Browser } from "./bindings/Browser";
export type { CounterStyle } from "./bindings/CounterStyle";
export type { CustomTheme } from "./bindings/CustomTheme";
export type { Provider } from "./bindings/Provider";
export type { ReviewItem } from "./bindings/ReviewItem";
export type { ReviewState } from "./bindings/ReviewState";
export type { Saved } from "./bindings/Saved";
export type { Theme } from "./bindings/Theme";
export type { Tier } from "./bindings/Tier";
export type { Tracker } from "./bindings/Tracker";
export type { ExtensionStatus, Member, Settings, Snapshot, Workspace };

/** Which PR, as people write it: provider plus the number shown as `!n` (Azure DevOps) or `#n` (GitHub). */
export type PrRef = Pick<Member, "repo" | "provider" | "number">;

/** What `saveSettings` takes: the settings without the workspaces, which have commands of their own. */
export type GeneralSettings = Omit<Settings, "workspaces" | "activeWorkspace">;

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  refresh: () => invoke<void>("refresh_now"),
  openList: () => invoke<void>("open_list"),
  hideList: () => invoke<void>("hide_list"),
  openPr: (id: number) => invoke<void>("open_pr", { id }),
  openDraft: (id: number) => invoke<void>("open_draft", { id }),
  openRow: (key: string) => invoke<void>("open_row", { key }),
  openTicket: (key: string) => invoke<void>("open_ticket", { key }),
  copyRow: (key: string) => invoke<void>("copy_row", { key }),
  setHidden: (key: string, hidden: boolean) => invoke<void>("set_hidden", { key, hidden }),
  savePat: (workspace: string, pat: string) => invoke<void>("save_pat", { workspace, pat }),
  saveGithubToken: (workspace: string, token: string) => invoke<void>("save_github_token", { workspace, token }),
  removePat: (workspace: string) => invoke<void>("remove_pat", { workspace }),
  removeGithubToken: (workspace: string) => invoke<void>("remove_github_token", { workspace }),
  saveSettings: (settings: GeneralSettings) => invoke<void>("save_settings", { settings }),
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
