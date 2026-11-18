import type { Member, PrRef, ReviewItem } from "./api";

/** "WebApp !3353" on Azure DevOps, "rado #4" on GitHub. */
export function prLabel(pr: PrRef): string {
  return `${pr.repo} ${pr.provider === "github" ? "#" : "!"}${pr.number}`;
}

export function age(hours: number): string {
  if (hours < 1) return "new";
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

export function ago(iso: string | null, now: Date): string {
  if (!iso) return "never";
  const seconds = Math.max(0, Math.round((now.getTime() - new Date(iso).getTime()) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  return `${Math.floor(minutes / 60)} h ago`;
}

/** Count at and above which the pill's number is fully red. */
export const HEAT_CAP = 10;

/** Heat colour for a count: blue (hue 225) at 0, through cyan, green and yellow, to red at HEAT_CAP or more. */
export function heat(count: number): string {
  const t = Math.min(Math.max(count, 0), HEAT_CAP) / HEAT_CAP;
  return `hsl(${Math.round(225 * (1 - t))}, 100%, 58%)`;
}

/**
 * The review to open next: of the rows waiting on your review, the one you opened least recently,
 * never-opened rows first in rank order. `openedAt` maps a row key to when you last opened it.
 */
export function nextReview(items: ReviewItem[], openedAt: ReadonlyMap<string, number>): ReviewItem | null {
  const reviews = items.filter((i) => i.actionable && i.state !== "mine");
  const opened = (i: ReviewItem) => openedAt.get(i.key) ?? 0;
  return reviews.reduce<ReviewItem | null>((best, i) => (best === null || opened(i) < opened(best) ? i : best), null);
}

/**
 * The row's hide button. Hiding takes every PR of the row that involves you, so a row holding a PR of yours says
 * "Hide" even when someone else's PR leads it; "Not mine" is only for rows of other people's PRs.
 */
export function hideButton(item: ReviewItem): { label: string; title: string } {
  if (item.hidden) return { label: "Show", title: "Show again" };
  return item.members.some((m) => m.state === "mine")
    ? { label: "Hide", title: "Hide until it closes" }
    : { label: "Not mine", title: "Not yours: leave the count until it closes" };
}

/** Rows ordered by their most recently opened PR, newest first. */
export function newestFirst(items: ReviewItem[]): ReviewItem[] {
  return [...items].sort((a, b) => a.newestAgeHours - b.newestAgeHours);
}

/** Counter size limits and button step, in percent; Rust clamps to the same range. */
export const SCALE_MIN = 75;
export const SCALE_MAX = 200;
export const SCALE_STEP = 25;

/** A typed counter size: any whole percent from SCALE_MIN to SCALE_MAX. */
export function parseScale(text: string): number | null {
  return parseWhole(text, [SCALE_MIN, SCALE_MAX]);
}

/** Allowed review list width and height in percent of its screen, matching `LIST_*_PERCENT_RANGE` in `store.rs`. */
export const LIST_WIDTH_PERCENT: readonly [number, number] = [20, 100];
export const LIST_HEIGHT_PERCENT: readonly [number, number] = [25, 100];

/** A typed whole number inside `range`, such as a list width in percent. */
export function parseWhole(text: string, [min, max]: readonly [number, number]): number | null {
  const trimmed = text.trim();
  // Number("") is 0, which would pass any range that starts at 0.
  const value = trimmed === "" ? NaN : Number(trimmed);
  return Number.isInteger(value) && value >= min && value <= max ? value : null;
}

/** List text size limits, button step and chips, in percent; Rust clamps to the same range. */
export const TEXT_SCALE: readonly [number, number] = [80, 150];
export const TEXT_STEP = 5;
export const TEXT_CHOICES = [90, 100, 115, 130] as const;

/** The − / + buttons: the next multiple of `step` down or up, so a typed 110 steps to 100 or 125 at step 25. */
export function stepScale(
  current: number,
  direction: -1 | 1,
  step = SCALE_STEP,
  [min, max]: readonly [number, number] = [SCALE_MIN, SCALE_MAX],
): number {
  const next = direction > 0
    ? (Math.floor(current / step) + 1) * step
    : (Math.ceil(current / step) - 1) * step;
  return Math.min(max, Math.max(min, next));
}

/** Hours from an ISO timestamp to `now`, never negative. */
export function hoursSince(iso: string, now: Date): number {
  return Math.max(0, Math.floor((now.getTime() - new Date(iso).getTime()) / 3_600_000));
}

/**
 * A row's times: opened, then last activity. Both are measured against `now`, so they stay consistent
 * between polls. The second part is absent when there was no activity after opening or it is unknown.
 */
export function ageColumn(createdAt: string, updatedAt: string | null, now: Date): { opened: string; active: string | null } {
  const phrase = (hours: number) => (hours < 1 ? "just now" : `${age(hours)} ago`);
  return {
    opened: `opened ${phrase(hoursSince(createdAt, now))}`,
    active: updatedAt === null ? null : `active ${phrase(hoursSince(updatedAt, now))}`,
  };
}

/** One phrase of a row's details line; `kind` names the ones with a colour of their own. */
export interface RowFact {
  text: string;
  kind?: "required" | "approved" | "objected";
  title?: string;
}

/** A row's details, in display order. */
export function rowFacts(item: ReviewItem): RowFact[] {
  const { approvals, objections, comments } = item;
  const facts: (RowFact | false | null)[] = [
    ...(item.members.length > 1 ? [{ text: `${item.members.length} PRs` }, { text: item.branch }] : []),
    { text: item.author },
    item.required && { text: "required", kind: "required" },
    approvals > 0 && { text: `✓ ${approvals}`, kind: "approved", title: `${approvals} approved` },
    objections > 0 && { text: `✗ ${objections}`, kind: "objected", title: `${objections} rejected or waiting for author` },
    comments && comments.total > 0 && {
      text: `${comments.total} comment${comments.total === 1 ? "" : "s"}${comments.openThreads > 0 ? ` (${comments.openThreads} open)` : ""}`,
    },
    item.draft && { text: "draft" },
    item.releaseTarget && { text: `→ ${item.target}`, title: `Merges into ${item.target}` },
  ];
  return facts.filter((f): f is RowFact => Boolean(f));
}

/** Hover text for a row's details: one line per PR with its exact opened and, when known, last-activity times. */
export function datesTooltip(members: Member[], format: (iso: string) => string): string {
  return members
    .map((m) => `${prLabel(m)}: opened ${format(m.createdAt)}${m.updatedAt ? ` · last activity ${format(m.updatedAt)}` : ""}`)
    .join("\n");
}

export function parseMinutes(text: string): number | null {
  const trimmed = text.trim();
  // Number("") is 0, which would pass any range that starts at 0.
  const value = trimmed === "" ? NaN : Number(trimmed);
  return Number.isInteger(value) && value >= 1 && value <= 120 ? value : null;
}
