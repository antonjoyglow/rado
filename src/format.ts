import type { Member, PrRef, ReviewItem } from "./api";

/** "WebApp !3353" on Azure DevOps, "rado #4" on GitHub. */
export function prLabel(pr: PrRef): string {
  return `${pr.repo} ${pr.provider === "github" ? "#" : "!"}${pr.number}`;
}

export function age(hours: number): string {
  if (hours < 1) return "new";
  if (hours < 48) return `${hours}h`;
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

/** The pill's two numbers: PRs waiting on your review, and your own PRs that need work. */
export function pillCounts(items: ReviewItem[]): { reviews: number; mine: number } {
  const actionable = items.filter(isActionable);
  const mine = actionable.filter((i) => i.state === "mine").length;
  return { reviews: actionable.length - mine, mine };
}

/** Count at and above which the pill's number is fully red. */
export const HEAT_CAP = 10;

/** Heat colour for a count: blue (hue 225) at 0, through cyan, green and yellow, to red at HEAT_CAP or more. */
export function heat(count: number): string {
  const t = Math.min(Math.max(count, 0), HEAT_CAP) / HEAT_CAP;
  return `hsl(${Math.round(225 * (1 - t))}, 100%, 58%)`;
}

export function isActionable(item: ReviewItem): boolean {
  return !item.hidden && item.actionable;
}

/**
 * The review to open next: of the rows waiting on your review, the one you opened least recently,
 * never-opened rows first in rank order. `openedAt` maps a row key to when you last opened it.
 */
export function nextReview(items: ReviewItem[], openedAt: ReadonlyMap<string, number>): ReviewItem | null {
  const reviews = items.filter((i) => isActionable(i) && i.state !== "mine");
  const opened = (i: ReviewItem) => openedAt.get(i.key) ?? 0;
  return reviews.reduce<ReviewItem | null>((best, i) => (best === null || opened(i) < opened(best) ? i : best), null);
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
  const value = Number(text.trim());
  return Number.isInteger(value) && value >= SCALE_MIN && value <= SCALE_MAX ? value : null;
}

/** Allowed review list width and height in px, matching `LIST_*_RANGE` in `store.rs`. */
export const LIST_WIDTH: readonly [number, number] = [360, 1600];
export const LIST_HEIGHT: readonly [number, number] = [400, 1600];

/** A typed list width or height: a whole number of px inside `range`. */
export function parseListSide(text: string, [min, max]: readonly [number, number]): number | null {
  const value = Number(text.trim());
  return Number.isInteger(value) && value >= min && value <= max ? value : null;
}

/** The − / + buttons: the next multiple of SCALE_STEP down or up, so a typed 110 steps to 100 or 125. */
export function stepScale(current: number, direction: -1 | 1): number {
  const next = direction > 0
    ? (Math.floor(current / SCALE_STEP) + 1) * SCALE_STEP
    : (Math.ceil(current / SCALE_STEP) - 1) * SCALE_STEP;
  return Math.min(SCALE_MAX, Math.max(SCALE_MIN, next));
}

/** Hours from an ISO timestamp to `now`, never negative. */
export function hoursSince(iso: string, now: Date): number {
  return Math.max(0, Math.floor((now.getTime() - new Date(iso).getTime()) / 3_600_000));
}

/**
 * The age column: opened, then last activity. Both are measured against `now`, so they stay consistent
 * between polls. The second part is absent when there was no activity after opening or it is unknown.
 */
export function ageColumn(createdAt: string, updatedAt: string | null, now: Date): { opened: string; active: string | null } {
  const phrase = (hours: number) => (hours < 1 ? "just now" : `${age(hours)} ago`);
  return {
    opened: `opened ${phrase(hoursSince(createdAt, now))}`,
    active: updatedAt === null ? null : `active ${phrase(hoursSince(updatedAt, now))}`,
  };
}

/** Hover text for the age column: one line per PR with its exact opened and, when known, last-activity times. */
export function datesTooltip(members: Member[], format: (iso: string) => string): string {
  return members
    .map((m) => `${prLabel(m)}: opened ${format(m.createdAt)}${m.updatedAt ? ` · last activity ${format(m.updatedAt)}` : ""}`)
    .join("\n");
}

export function parseMinutes(text: string): number | null {
  const value = Number(text.trim());
  return Number.isInteger(value) && value >= 1 && value <= 120 ? value : null;
}
