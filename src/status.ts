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
  const stale = isStale(s.lastRefreshed, s.settings.pollMinutes, now);
  if (s.error) return { kind: "error", text: stale ? `${s.error} · last checked ${ago(s.lastRefreshed, now)}` : s.error };
  if (stale) return { kind: "stale", text: `Last checked ${ago(s.lastRefreshed, now)}` };
  return null;
}

/** The banner shows only when one of its lines has something to say. */
export function showStatus(line: StatusLine | null, warning: string | null, actionError: string | null, newerRelease: unknown): boolean {
  return line !== null || Boolean(warning) || Boolean(actionError) || Boolean(newerRelease);
}

/**
 * Runs actions taken in the list (open, hide, switch workspace). Each clears the last one's error as it starts and
 * reports its own failure, unless a newer action has started since.
 */
export function listActions(report: (error: string | null) => void) {
  let latest = 0;
  return (action: Promise<unknown>): Promise<void> => {
    const run = ++latest;
    report(null);
    return action.then(() => undefined, (e: unknown) => {
      if (run === latest) report(String(e));
    });
  };
}

export function checkedText(lastRefreshed: string | null, now: Date): string {
  return lastRefreshed ? `checked ${ago(lastRefreshed, now)}` : "not checked yet";
}

export function refreshTitle(lastRefreshed: string | null, pollMinutes: number, now: Date): string {
  return `Refresh now · ${checkedText(lastRefreshed, now)} · every ${pollMinutes} min`;
}

/** The counter's `!`: no token, or an error while the data never loaded or is stale. */
export function counterProblem(s: Pick<Snapshot, "hasPat" | "hasGithubToken" | "error" | "lastRefreshed">, pollMinutes: number, now: Date): boolean {
  if (!s.hasPat && !s.hasGithubToken) return true;
  return s.error !== null && (!s.lastRefreshed || isStale(s.lastRefreshed, pollMinutes, now));
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

/** New reviews owed a yawn: `held` waits for the list's focus, `play` yawns now. */
export type Yawn = { held: number; play: number };

export function onFresh(held: number, count: number, focused: boolean): Yawn {
  return focused ? { held: 0, play: held + count } : { held: held + count, play: 0 };
}

export function onFocus(held: number): Yawn {
  return { held: 0, play: held };
}

/** A workspace switch drops the held yawn: its reviews belong to the workspace no longer shown. */
export function onSnapshot(held: number, prevWorkspace: string | null, nextWorkspace: string): number {
  return prevWorkspace !== null && prevWorkspace !== nextWorkspace ? 0 : held;
}
