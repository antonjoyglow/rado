import { describe, expect, it } from "vitest";
import { DROPPED, lastingSaver, serialSaver } from "./save";

type S = { a: number; b: number };

/** A backend whose saves resolve or fail when the test says, recording what each one sent. */
function backend() {
  const calls: { value: S; ok: () => void; fail: (e: string) => void }[] = [];
  const send = (value: S) => new Promise<void>((resolve, reject) => calls.push({ value, ok: resolve, fail: (e) => reject(new Error(e)) }));
  const tick = () => new Promise((r) => setTimeout(r, 0));
  return { calls, send, tick };
}

describe("serialSaver", () => {
  it("sends one save at a time, each with the values current when it is sent", async () => {
    const { calls, send, tick } = backend();
    const settings: S = { a: 0, b: 0 };
    const save = serialSaver(settings, send, () => {});
    settings.a = 1;
    const first = save(settings);
    await tick();
    settings.b = 2;
    const second = save(settings);
    await tick();
    expect(calls.map((c) => c.value)).toEqual([{ a: 1, b: 0 }]);
    calls[0].ok();
    await first;
    await tick();
    expect(calls.map((c) => c.value)).toEqual([{ a: 1, b: 0 }, { a: 1, b: 2 }]);
    calls[1].ok();
    await expect(second).resolves.toBeUndefined();
  });

  it("on a failure reverts to the last saved value and drops the saves queued behind it", async () => {
    const { calls, send, tick } = backend();
    const settings: S = { a: 0, b: 0 };
    const reverts: S[] = [];
    const save = serialSaver(settings, send, (saved) => {
      reverts.push(saved);
      Object.assign(settings, saved);
    });
    settings.a = 1;
    const ok = save(settings);
    await tick();
    calls[0].ok();
    await ok;

    settings.b = 5;
    const failing = save(settings);
    settings.a = 9;
    const queued = save(settings);
    await tick();
    calls[1].fail("disk full");
    await expect(failing).rejects.toThrow("disk full");
    await expect(queued).rejects.toBe(DROPPED);
    expect(calls).toHaveLength(2);
    expect(reverts).toEqual([{ a: 1, b: 0 }]);
    expect(settings).toEqual({ a: 1, b: 0 });

    settings.b = 3;
    const after = save(settings);
    await tick();
    expect(calls[2].value).toEqual({ a: 1, b: 3 });
    calls[2].ok();
    await expect(after).resolves.toBeUndefined();
  });

  it("reverts to the starting value when the first save fails", async () => {
    const { calls, send, tick } = backend();
    const settings: S = { a: 0, b: 0 };
    let reverted: S | null = null;
    const save = serialSaver(settings, send, (saved) => (reverted = saved));
    settings.a = 4;
    const failing = save(settings);
    await tick();
    calls[0].fail("locked");
    await expect(failing).rejects.toThrow("locked");
    expect(reverted).toEqual({ a: 0, b: 0 });
  });
});

describe("lastingSaver", () => {
  it("lets a second view queue behind a running save and start from the value being saved", async () => {
    const { calls, send, tick } = backend();
    const saver = lastingSaver<S>({ a: 0, b: 0 }, send);
    const first: S = { a: 0, b: 0 };
    first.a = 1;
    const running = saver.save(first);
    await tick();

    const second = { ...saver.latest };
    expect(second).toEqual({ a: 1, b: 0 });
    second.b = 2;
    const queued = saver.save(second);
    await tick();
    expect(calls.length).toBe(1);

    calls[0].ok();
    await running;
    await tick();
    calls[1].ok();
    await queued;
    expect(calls.map((c) => c.value)).toEqual([{ a: 1, b: 0 }, { a: 1, b: 2 }]);
  });

  it("on a failure calls the latest view's revert and resets latest to the saved value", async () => {
    const { calls, send, tick } = backend();
    const saver = lastingSaver<S>({ a: 0, b: 0 }, send);
    const reverts: S[] = [];
    saver.revert = (saved) => reverts.push(saved);
    const failing = saver.save({ a: 5, b: 0 });
    const dropped = saver.save({ a: 5, b: 6 });
    await tick();
    calls[0].fail("locked");
    await expect(failing).rejects.toThrow("locked");
    await expect(dropped).rejects.toBe(DROPPED);
    expect(reverts).toEqual([{ a: 0, b: 0 }]);
    // The next view starts from the saved value, not the dropped one.
    expect(saver.latest).toEqual({ a: 0, b: 0 });
  });
});
