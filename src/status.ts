import type { Settings, Snapshot } from "./api";
import { ago } from "./format";

type Polled = Pick<Snapshot, "polling" | "error" | "lastRefreshed"> & { settings: Pick<Settings, "pollMinutes" | "activeWorkspace"> };

/** A refresh shows its busy line only once it runs this long; quicker ones show only on the ↻ button. */
export const BUSY_AFTER_MS = 5_000;

/** Stale once the last successful refresh is older than two missed polls plus a minute. */
export function isStale(lastRefreshed: string | null, pollMinutes: number, now: Date): boolean {
  if (!lastRefreshed) return false;
  return now.getTime() - new Date(lastRefreshed).getTime() > (2 * pollMinutes + 1) * 60_000;
}

export type StatusLine = { kind: "busy" | "error" | "stale"; text: string };

/** The status banner's main line, or null when there is nothing to say. */
export function statusLine(s: Polled, pollingSince: number | null, now: Date): StatusLine | null {
  const busyMs = s.polling && pollingSince !== null ? now.getTime() - pollingSince : 0;
  if (busyMs >= BUSY_AFTER_MS) return { kind: "busy", text: `Refreshing… usually 1–3 s (${Math.round(busyMs / 1000)} s)` };
  if (s.error) return { kind: "error", text: s.error };
  if (isStale(s.lastRefreshed, s.settings.pollMinutes, now)) return { kind: "stale", text: `Last checked ${ago(s.lastRefreshed, now)}` };
  return null;
}

/** The banner shows only when one of its lines has something to say. */
export function showStatus(line: StatusLine | null, warning: string | null, newerRelease: unknown): boolean {
  return line !== null || Boolean(warning) || Boolean(newerRelease);
}

export function refreshTitle(lastRefreshed: string | null, pollMinutes: number, now: Date): string {
  const checked = lastRefreshed ? `checked ${ago(lastRefreshed, now)}` : "not checked yet";
  return `Refresh now · ${checked} · every ${pollMinutes} min`;
}

/**
 * Whether `next` is a successful refresh of the workspace `prev` showed. `polling` going false is not the signal:
 * it also clears when a refresh fails, while `lastRefreshed` moves only on success.
 */
export function glows(prev: Polled | null, next: Polled): boolean {
  return prev !== null
    && prev.settings.activeWorkspace === next.settings.activeWorkspace
    && next.error === null
    && next.lastRefreshed !== null
    && next.lastRefreshed !== prev.lastRefreshed;
}

export function newReviewsText(count: number): string {
  return `${count} new review${count === 1 ? "" : "s"}`;
}
