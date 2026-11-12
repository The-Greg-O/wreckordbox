import { describe, expect, it } from "vitest";

import { TrackWrites } from "./trackWrites";

/** A promise and the means to settle it from the test. */
function held<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
}

describe("TrackWrites", () => {
  it("settles a track once every write of it, including one added while waiting, has landed", async () => {
    const writes = new TrackWrites();
    const first = held<string | null>();
    const second = held<string | null>();
    writes.track("1", "cues", first.promise);
    let done = false;
    const settled = writes.settled("1").then((why) => { done = true; return why; });
    writes.track("1", "grid", second.promise);
    first.resolve(null);
    await Promise.resolve();
    await Promise.resolve();
    expect(done).toBe(false);
    second.resolve(null);
    expect(await settled).toBeNull();
  });

  it("does not wait for another track's writes", async () => {
    const writes = new TrackWrites();
    writes.track("2", "grid", new Promise(() => {}));
    expect(await writes.settled("1")).toBeNull();
  });

  it("keeps a failure against the track it came from until a later write of that kind lands", async () => {
    const writes = new TrackWrites();
    writes.track("1", "grid", Promise.resolve("The beat grid is locked."));
    expect(await writes.settled("1")).toBe("grid: The beat grid is locked.");
    expect(await writes.settled("2")).toBeNull();
    expect(writes.failures()).toEqual([{ track: "1", kind: "grid", message: "The beat grid is locked." }]);
    // A cue write landing does not clear the grid's failure.
    writes.track("1", "cues", Promise.resolve(null));
    expect(await writes.settled("1")).toBe("grid: The beat grid is locked.");
    writes.track("1", "grid", Promise.resolve(null));
    expect(await writes.settled("1")).toBeNull();
  });

  it("tells its listeners when a write starts, ends or fails, and forgets a dismissed failure", async () => {
    const writes = new TrackWrites();
    let heard = 0;
    const stop = writes.subscribe(() => { heard += 1; });
    writes.track("1", "cues", Promise.resolve("refused"));
    await writes.settled("1");
    expect(heard).toBeGreaterThanOrEqual(2);
    expect(writes.pending("1")).toBe(false);
    writes.dismiss("1", "cues");
    expect(writes.failures()).toEqual([]);
    stop();
  });
});
