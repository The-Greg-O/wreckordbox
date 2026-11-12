import { describe, expect, it, vi } from "vitest";

import type { DeviceFormat } from "@/ipc/types";
import { prepareDevicePlaylistDrop } from "./devicePlaylistDrop";

const format: DeviceFormat = "deviceLibrary";

describe("prepareDevicePlaylistDrop", () => {
  it("exports only missing tracks, maps them to device ids, then honors Skip duplicates", async () => {
    const backend = {
      devicePlaylistCollectionTracks: vi.fn()
        .mockResolvedValueOnce(["device-1", null, "device-2"])
        .mockResolvedValueOnce(["device-1", "device-3", "device-2"]),
      devicePlaylistDuplicates: vi.fn().mockResolvedValue(["device-1"]),
      exportTracksToDevice: vi.fn().mockResolvedValue({}),
    };
    const ask = vi.fn().mockResolvedValue(false);
    const approve = vi.fn().mockResolvedValue("token");

    const prepared = await prepareDevicePlaylistDrop(
      backend, "/Volumes/TEST", format, "7", ["100", "101", "102"], undefined, undefined, ask, "Set", approve,
    );

    expect(ask).toHaveBeenCalledWith(1, 3, "Set");
    // Approved before anything is written: one to export, two going in.
    expect(approve).toHaveBeenCalledWith(["101"], 2, false);
    expect(backend.exportTracksToDevice).toHaveBeenCalledWith(["101"], "/Volumes/TEST", undefined, undefined, "token");
    expect(prepared).toEqual({ edit: { kind: "add", playlist: "7", tracks: ["device-3", "device-2"], duplicates: false }, approval: "token" });
  });

  it("a write not approved exports nothing and adds nothing", async () => {
    const backend = {
      devicePlaylistCollectionTracks: vi.fn().mockResolvedValue(["device-1", null]),
      devicePlaylistDuplicates: vi.fn().mockResolvedValue([]),
      exportTracksToDevice: vi.fn(),
    };
    const prepared = await prepareDevicePlaylistDrop(
      backend, "/Volumes/TEST", format, "7", ["100", "101"], undefined, undefined,
      vi.fn().mockResolvedValue(true), "Set", vi.fn().mockResolvedValue(null),
    );
    expect(prepared).toBeNull();
    expect(backend.exportTracksToDevice).not.toHaveBeenCalled();
    expect(backend.devicePlaylistCollectionTracks).toHaveBeenCalledTimes(1);
  });

  it("cancelling the duplicate prompt does not export tracks", async () => {
    const backend = {
      devicePlaylistCollectionTracks: vi.fn().mockResolvedValue([null]),
      devicePlaylistDuplicates: vi.fn().mockResolvedValue([]),
      exportTracksToDevice: vi.fn(),
    };

    const edit = await prepareDevicePlaylistDrop(
      backend, "/Volumes/TEST", format, "7", ["100"], undefined, undefined,
      vi.fn().mockResolvedValue(null), "Set", vi.fn().mockResolvedValue("token"),
    );

    expect(edit).toBeNull();
    expect(backend.exportTracksToDevice).not.toHaveBeenCalled();
  });

  it("refuses to add when the export did not produce a device-library row", async () => {
    const backend = {
      devicePlaylistCollectionTracks: vi.fn().mockResolvedValueOnce([null]).mockResolvedValueOnce([null]),
      devicePlaylistDuplicates: vi.fn().mockResolvedValue([]),
      exportTracksToDevice: vi.fn().mockResolvedValue({}),
    };

    await expect(prepareDevicePlaylistDrop(
      backend, "/Volumes/TEST", format, "7", ["100"], undefined, undefined,
      vi.fn().mockResolvedValue(false), "Set", vi.fn().mockResolvedValue("token"),
    )).rejects.toThrow("Only tracks of this library on the device can go into its playlists.");
  });
});
