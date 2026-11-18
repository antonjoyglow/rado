export const DROPPED = "Not saved: an earlier change failed to save, so the settings went back to the last saved values.";

/**
 * Saves one shared settings object, one save at a time, each sending the object as it is when its turn comes.
 * A failed save hands `revert` a copy of the last saved value, and saves queued before that failure reject with
 * DROPPED instead of sending, so no save ever carries a change whose save failed.
 */
export function serialSaver<T>(initial: T, send: (value: T) => Promise<void>, revert: (saved: T) => void): (current: T) => Promise<void> {
  let saved = structuredClone(initial);
  let queue: Promise<unknown> = Promise.resolve();
  let failures = 0;
  return (current) => {
    const seen = failures;
    const run = queue.then(async () => {
      // A plain string, like the errors from the Rust commands, so notes show it as is.
      if (seen !== failures) throw DROPPED;
      const sent = structuredClone(current);
      try {
        await send(sent);
        saved = sent;
      } catch (e) {
        failures++;
        revert(structuredClone(saved));
        throw e;
      }
    });
    queue = run.catch(() => {});
    return run;
  };
}

/** A serialSaver that outlives one view: a later view starts from `latest` while saves are pending, and sets `revert`. */
export interface LastingSaver<T> {
  save: (current: T) => Promise<void>;
  /** Saves queued or running. */
  pending: number;
  /** The value of the last save queued, or the last saved value after a failure. */
  latest: T;
  revert: (saved: T) => void;
}

export function lastingSaver<T>(initial: T, send: (value: T) => Promise<void>): LastingSaver<T> {
  const saver: LastingSaver<T> = { pending: 0, latest: structuredClone(initial), revert: () => {}, save: () => Promise.resolve() };
  const queue = serialSaver(initial, send, (saved) => {
    saver.latest = structuredClone(saved);
    saver.revert(saved);
  });
  saver.save = (current) => {
    saver.pending++;
    saver.latest = structuredClone(current);
    return queue(current).finally(() => saver.pending--);
  };
  return saver;
}
