export const DROPPED = "Not saved: an earlier change failed to save, so the settings went back to the last saved values.";

/**
 * Saves one shared settings object, one save at a time, each sending the object as it is when its turn comes.
 * `send` resolves with the value as stored, which the backend can have cleaned, and each save resolves with it. A failed
 * save hands `revert` a copy of the last saved value, and saves queued before that failure reject with DROPPED instead
 * of sending, so no save ever carries a change whose save failed.
 */
export function serialSaver<T>(initial: T, send: (value: T) => Promise<T>, revert: (saved: T) => void): (current: T) => Promise<T> {
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
        saved = await send(sent);
        return structuredClone(saved);
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

/** A serialSaver that outlives one view: a later view starts from `latest` and sets `revert`. */
export interface LastingSaver<T> {
  save: (current: T) => Promise<T>;
  /** The value of the last save queued, as saved once it is, or the last saved value after a failure. */
  latest: T;
  revert: (saved: T) => void;
}

export function lastingSaver<T>(initial: T, send: (value: T) => Promise<T>): LastingSaver<T> {
  const saver: LastingSaver<T> = { latest: structuredClone(initial), revert: () => {}, save: () => Promise.resolve(initial) };
  const queue = serialSaver(initial, send, (saved) => {
    saver.latest = structuredClone(saved);
    saver.revert(saved);
  });
  saver.save = (current) => {
    const queued = (saver.latest = structuredClone(current));
    return queue(current).then((saved) => {
      if (saver.latest === queued) saver.latest = saved;
      return saved;
    });
  };
  return saver;
}

/**
 * Saves a typed secret. One paste can commit twice: Enter runs its key handler and change, and a click on Save
 * follows the change of leaving the field. Blank text, and text already being saved, are skipped. Resolves true
 * once saved, false when skipped.
 */
export function secretSaver(store: (value: string) => Promise<void>): (value: string) => Promise<boolean> {
  let saving: string | null = null;
  return (value) => {
    if (!value.trim() || value === saving) return Promise.resolve(false);
    saving = value;
    return store(value).then(() => true).finally(() => {
      if (saving === value) saving = null;
    });
  };
}
