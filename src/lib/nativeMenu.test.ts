import { expect, it } from "vitest";

import { nativeMenuLabels } from "./nativeMenu";

it("sends every native menu label through the active translator", () => {
  const labels = nativeMenuLabels((source) => `translated:${source}`);

  expect(labels.File).toBe("translated:File");
  expect(labels.Edit).toBe("translated:Edit");
  expect(labels.Track).toBe("translated:Track");
  expect(labels["Analyze Track"]).toBe("translated:Analyze Track");
  expect(labels["Locate track loaded on deck"]).toBe("translated:Locate track loaded on deck");
  expect(labels.View).toBe("translated:View");
  expect(labels).not.toHaveProperty("Help");
  expect(labels).not.toHaveProperty("Report bug…");
  expect(labels).not.toHaveProperty("Check for Updates…");
  expect(labels.Playlist).toBe("translated:Playlist");
  expect(labels["Sort Items"]).toBe("translated:Sort Items");
  expect(labels["Analysis Lock On"]).toBe("translated:Analysis Lock On");
  expect(labels["Export a playlist to a file for music apps (*.m3u8)"])
    .toBe("translated:Export a playlist to a file for music apps (*.m3u8)");
  expect(Object.keys(labels)).toHaveLength(53);
});
