import { describe, expect, it } from "vitest";

import { menuCommand, resolveMenu } from "./menu";

describe("menu", () => {
  it("ignores an id that is not ours", () => {
    expect(menuCommand("quit")).toBeNull();
    expect(resolveMenu("quit", false)).toBeNull();
  });

  it("locates the loaded track from the Track menu, whatever the library state", () => {
    expect(menuCommand("locate-loaded")?.writes).toBe(false);
    expect(resolveMenu("locate-loaded", true)).toEqual({ action: "locate-loaded" });
  });

  it("leaves Track › Analyze Track to the command, which refuses a read-only library itself", () => {
    expect(menuCommand("analyze-track")?.writes).toBe(false);
    expect(resolveMenu("analyze-track", true)).toEqual({ action: "analyze-track" });
  });

  it("hands a Track or Playlist menu item to its shared command", () => {
    expect(resolveMenu("relocate", false)).toEqual({
      action: "bar-command",
      item: { id: "relocate", action: "relocate", owner: "list", writes: true },
    });
    expect(resolveMenu("export-playlist-m3u8", true)).toMatchObject({
      action: "bar-command",
      item: { action: "exportPlaylistM3u8" },
    });
  });

  it("refuses a Track or Playlist menu write on a protected library, as rekordbox does", () => {
    expect(resolveMenu("sort-items", false)).toMatchObject({
      action: "bar-command", item: { action: "sortItems", scope: "global" },
    });
    expect(resolveMenu("sort-items", true, true)).toEqual({
      refused: "Editing is locked by Library Protection. Turn it off in Preferences to edit.",
    });
    expect(resolveMenu("delete-playlist", true, true)).toEqual({
      refused: "Editing is locked by Library Protection. Turn it off in Preferences to edit.",
    });
    expect(resolveMenu("remove-from-collection", true)).toEqual({
      refused: "Editing is locked while rekordbox is running. Quit rekordbox to enable editing.",
    });
  });

  it("runs an action that only reads", () => {
    expect(resolveMenu("settings", true)).toEqual({ action: "settings" });
  });

  it("refuses a write while rekordbox holds the library", () => {
    expect(resolveMenu("import", true)).toEqual({
      refused: "Editing is locked while rekordbox is running. Quit rekordbox to enable editing.",
    });
    expect(resolveMenu("import", false)).toEqual({ action: "import" });
  });

  it("treats importing a folder as the write it is", () => {
    expect(menuCommand("import-folder")?.writes).toBe(true);
    expect(resolveMenu("import-folder", true)).toEqual({
      refused: "Editing is locked while rekordbox is running. Quit rekordbox to enable editing.",
    });
    expect(resolveMenu("import-folder", false)).toEqual({ action: "import-folder" });
  });

  it("has no update check and no bug report to offer", () => {
    // Wreckordbox contacts none of upstream's services, so an id for either
    // is not one of its commands.
    expect(resolveMenu("updates", false)).toBeNull();
    expect(resolveMenu("report-bug", false)).toBeNull();
  });

  it("opens the Missing File Manager on a read-only library", () => {
    // Its list only reads; Auto Relocate, Relocate and Delete are greyed.
    expect(resolveMenu("missing", true)).toEqual({ action: "missing" });
  });

  it("says why rather than doing nothing", () => {
    const outcome = resolveMenu("import", true);
    expect(outcome).not.toBeNull();
    expect(outcome).toHaveProperty("refused");
  });
});
