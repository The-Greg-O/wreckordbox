import { describe, expect, it, vi } from "vitest";

import { askDevicePlaylistDrop } from "./devicePlaylistDrop";

const backend = (rows: (string | null)[], held: string[]) => ({
  devicePlaylistCollectionTracks: vi.fn(() => Promise.resolve(rows)),
  devicePlaylistDuplicates: vi.fn(() => Promise.resolve(held)),
});

describe("askDevicePlaylistDrop", () => {
  it("reads which tracks the Stick and the playlist hold, asks, and writes nothing", async () => {
    const reads = backend(["row:1", null, "row:3"], ["row:3"]);
    const ask = vi.fn(() => Promise.resolve(false));
    const drop = await askDevicePlaylistDrop(reads, "/Volumes/TEST", "deviceLibrary", "7", ["1", "2", "3"], ask, "Friday");
    expect(ask).toHaveBeenCalledWith(1, 3, "Friday");
    expect(reads.devicePlaylistDuplicates).toHaveBeenCalledWith("/Volumes/TEST", "deviceLibrary", "7", ["row:1", "row:3"]);
    expect(drop).toEqual({ tracks: ["1", "2", "3"], duplicates: false, adding: 2, missing: 1 });
  });

  it("adds every track when the DJ answers Add", async () => {
    const drop = await askDevicePlaylistDrop(backend(["row:1"], ["row:1"]), "/s", "oneLibrary", "7", ["1"], () => Promise.resolve(true), "x");
    expect(drop).toEqual({ tracks: ["1"], duplicates: true, adding: 1, missing: 0 });
  });

  it("stops when the question is cancelled or nothing would be added", async () => {
    expect(await askDevicePlaylistDrop(backend(["row:1"], []), "/s", "oneLibrary", "7", ["1"], () => Promise.resolve(null), "x")).toBeNull();
    expect(await askDevicePlaylistDrop(backend(["row:1"], ["row:1"]), "/s", "oneLibrary", "7", ["1"], () => Promise.resolve(false), "x")).toBeNull();
  });

  it("refuses an answer that does not cover every track", async () => {
    await expect(askDevicePlaylistDrop(backend([null], []), "/s", "oneLibrary", "7", ["1", "2"], () => Promise.resolve(true), "x"))
      .rejects.toThrow("Only tracks of this library");
  });
});
