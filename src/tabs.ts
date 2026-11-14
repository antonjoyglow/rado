import type { ReviewItem, ReviewState } from "./api";

export type SectionKey = ReviewState | "hidden";

/** The list's tabs, in strip order. `label` fits the 440 px Compact window; `title` is the tooltip. */
export const SECTIONS: readonly { key: SectionKey; label: string; title: string }[] = [
  { key: "needsReview", label: "Review", title: "Needs your review" },
  { key: "reReview", label: "Re-review", title: "New pushes since your vote" },
  { key: "mine", label: "Mine", title: "Your PRs" },
  { key: "waitingOnAuthor", label: "Author", title: "Waiting on author" },
  { key: "approved", label: "Approved", title: "Approved by you or your team" },
  { key: "stale", label: "Quiet", title: "Quiet for 3 weeks" },
  { key: "hidden", label: "Hidden", title: "Not mine or hidden" },
];

export type SettingsTab = "ado" | "github" | "general";

export const SETTINGS_TABS: readonly { key: SettingsTab; label: string }[] = [
  { key: "ado", label: "Azure DevOps" },
  { key: "github", label: "GitHub" },
  { key: "general", label: "General" },
];

export const inSection = (item: ReviewItem, key: SectionKey) => (key === "hidden" ? item.hidden : !item.hidden && item.state === key);

/** The tab to open when none was remembered: the first of review, re-review and mine with rows, else review. */
export function defaultListTab(items: ReviewItem[]): SectionKey {
  const first: SectionKey[] = ["needsReview", "reReview", "mine"];
  return first.find((key) => items.some((i) => inSection(i, key))) ?? "needsReview";
}

/** A remembered tab if it is still one of `keys`, else null. */
export function restoreTab<K extends string>(stored: string | null, keys: readonly K[]): K | null {
  return keys.find((k) => k === stored) ?? null;
}

/** The tab index a key moves to from `index` among `count` tabs; arrows wrap. Null for any other key. */
export function stepTab(index: number, key: string, count: number): number | null {
  switch (key) {
    case "ArrowRight": return (index + 1) % count;
    case "ArrowLeft": return (index - 1 + count) % count;
    case "Home": return 0;
    case "End": return count - 1;
    default: return null;
  }
}
