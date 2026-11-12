import { describe, expect, it } from "vitest";

import type { TagPanel } from "./tagPanel";
import { TagStore, type TagBackend, type TagWrite } from "./tagStore";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/** A fake service: one track per id, dimensions `feel` (Fact) and `deck` (MyTag), versions bumped by each write. */
function fakeService() {
  const held: Record<string, Record<string, string[]>> = {};
  const versions: Record<string, number> = {};
  const writes: TagWrite[] = [];
  const holds: (() => void)[] = [];
  let holding = false;
  const failNext: Record<string, string> = {};
  const panel = (track: string): TagPanel => ({
    source: "tag_schema",
    schemaProblem: null,
    schemaVersion: "s1",
    dimensions: [
      { id: "deck", label: "Deck", deck: true, tree: false, tagParents: false, single: false, edit: "my_tag", tags: [
        { id: "a", name: "A", parent: null, depth: 0, myTag: "1" }, { id: "b", name: "B", parent: null, depth: 0, myTag: "2" }] },
      { id: "feel", label: "Feel", deck: false, tree: false, tagParents: false, single: false, edit: "fact", tags: [
        { id: "dark", name: "Dark", parent: null, depth: 0, myTag: null }, { id: "bright", name: "Bright", parent: null, depth: 0, myTag: null }] },
      { id: "vocals", label: "Vocals", deck: false, tree: false, tagParents: false, single: true, edit: "fact", tags: [
        { id: "male", name: "Male", parent: null, depth: 0, myTag: null }, { id: "female", name: "Female", parent: null, depth: 0, myTag: null }] },
    ],
    track: {
      id: track,
      set: { deck: [...(held[track]?.deck ?? [])], feel: [...(held[track]?.feel ?? [])], vocals: [...(held[track]?.vocals ?? [])] },
      unmapped: {},
      versions: Object.fromEntries(["deck", "feel", "vocals"].map((d) => [d, `${track}:${d}:${versions[`${track}:${d}`] ?? 0}`])),
      factsUnavailable: null,
    },
  });
  const backend: TagBackend = {
    read: async (track) => { await tick(); return panel(track); },
    write: async (write) => {
      writes.push(write);
      if (holding) await new Promise<void>((resolve) => holds.push(resolve));
      await tick();
      const key = `${write.track}:${write.dimension}`;
      const failure = failNext[key];
      if (failure !== undefined) {
        delete failNext[key];
        throw Object.assign(new Error(failure), { detail: failure === "stale" ? "tag_panel_stale" : "write_refused" });
      }
      if (write.version !== `${key}:${versions[key] ?? 0}`) throw Object.assign(new Error("stale"), { detail: "tag_panel_stale" });
      held[write.track] = { ...(held[write.track] ?? {}), [write.dimension]: [...write.terms] };
      versions[key] = (versions[key] ?? 0) + 1;
      return { panel: panel(write.track), factsNotRecorded: [] };
    },
  };
  return {
    backend, writes, held,
    hold: () => { holding = true; },
    release: () => { holding = false; for (const go of holds.splice(0)) go(); },
    failNext: (track: string, dimension: string, message: string) => { failNext[`${track}:${dimension}`] = message; },
  };
}

async function ready(store: TagStore, track: string) {
  store.refresh(track);
  for (let i = 0; i < 10 && !store.panel(track).panel; i += 1) await tick();
}

const dim = (store: TagStore, track: string, id: string) => store.panel(track).panel!.dimensions.find((d) => d.id === id)!;

describe("TagStore", () => {
  it("shows a click at once and writes it", async () => {
    const fake = fakeService();
    const store = new TagStore(fake.backend);
    await ready(store, "t1");
    store.toggle("t1", dim(store, "t1", "feel"), "dark");
    expect(store.shown("t1", dim(store, "t1", "feel"))).toEqual(["dark"]);
    expect(await store.settled("t1")).toBeNull();
    expect(fake.held.t1?.feel).toEqual(["dark"]);
  });

  it("clicks behind a write on its way become one write, which ends where the DJ left it", async () => {
    const fake = fakeService();
    const store = new TagStore(fake.backend);
    await ready(store, "t1");
    fake.hold();
    const feel = dim(store, "t1", "feel");
    store.toggle("t1", feel, "dark");
    await tick();
    store.toggle("t1", feel, "bright");
    store.toggle("t1", feel, "dark");
    store.toggle("t1", feel, "dark");
    expect(store.shown("t1", feel)).toEqual(["dark", "bright"]);
    fake.release();
    await store.settled("t1");
    fake.release();
    expect(await store.settled("t1")).toBeNull();
    expect(fake.writes.length).toBe(2);
    expect(fake.held.t1?.feel).toEqual(["dark", "bright"]);
  });

  it("keeps a failure for its track and dimension: an unrelated success never clears it", async () => {
    const fake = fakeService();
    const store = new TagStore(fake.backend);
    await ready(store, "t1");
    fake.failNext("t1", "feel", "Editing is locked");
    store.toggle("t1", dim(store, "t1", "feel"), "dark");
    store.toggle("t1", dim(store, "t1", "deck"), "a");
    await store.settled("t1");
    expect(fake.held.t1?.deck).toEqual(["a"]);
    const failures = store.failures();
    expect(failures).toHaveLength(1);
    expect(failures[0]).toMatchObject({ track: "t1", dimension: "feel", message: "Editing is locked" });
    // The refused click is not shown as made.
    expect(store.shown("t1", dim(store, "t1", "feel"))).toEqual([]);
    // Another track's success leaves it too.
    await ready(store, "t2");
    store.toggle("t2", dim(store, "t2", "feel"), "bright");
    await store.settled("t2");
    expect(store.failures()).toHaveLength(1);
    store.acknowledge(failures[0]!.key);
    expect(store.failures()).toHaveLength(0);
  });

  it("retrying a failure writes what the DJ asked for, from the panel as it is now, and clears it", async () => {
    const fake = fakeService();
    const store = new TagStore(fake.backend);
    await ready(store, "t1");
    fake.failNext("t1", "feel", "stale");
    store.toggle("t1", dim(store, "t1", "feel"), "dark");
    expect(await store.settled("t1")).toContain("stale");
    store.retry(store.failures()[0]!.key);
    expect(await store.settled("t1")).toBeNull();
    expect(fake.held.t1?.feel).toEqual(["dark"]);
    expect(store.failures()).toHaveLength(0);
  });

  it("settled answers why a track's tags were not saved, so a mark can stop", async () => {
    const fake = fakeService();
    const store = new TagStore(fake.backend);
    await ready(store, "t1");
    fake.hold();
    fake.failNext("t1", "deck", "Not allowed");
    store.toggle("t1", dim(store, "t1", "deck"), "b");
    let done = false;
    const waiting = store.settled("t1").then((why) => { done = true; return why; });
    await tick();
    expect(done).toBe(false);
    fake.release();
    expect(await waiting).toContain("Not allowed");
  });

  it("a single-tag dimension replaces its tag and clears it", async () => {
    const fake = fakeService();
    const store = new TagStore(fake.backend);
    await ready(store, "t1");
    const vocals = dim(store, "t1", "vocals");
    store.toggle("t1", vocals, "male");
    store.toggle("t1", vocals, "female");
    expect(store.shown("t1", vocals)).toEqual(["female"]);
    await store.settled("t1");
    expect(fake.held.t1?.vocals).toEqual(["female"]);
    store.toggle("t1", dim(store, "t1", "vocals"), "female");
    await store.settled("t1");
    expect(fake.held.t1?.vocals).toEqual([]);
  });
});
