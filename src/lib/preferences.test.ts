import { describe, expect, it } from "vitest";

import {
  BPM_RANGES,
  bpmRangeLimits,
  browseListVars,
  browseScale,
  BROWSE_SCALE_DEFAULT,
  DEFAULT_PREFERENCES,
  formatKey,
  quantizeFraction,
  sanitisePreferences,
  stepUiScale,
  uiScale,
  UI_SCALE_DEFAULT,
  UI_SCALES,
} from "./preferences";

describe("sanitisePreferences", () => {
  it("keeps the chosen output device and treats anything else as the system default (#799)", () => {
    expect(DEFAULT_PREFERENCES.audio.outputDevice).toBeNull();
    expect(sanitisePreferences({ audio: {} }).audio.outputDevice).toBeNull();
    expect(sanitisePreferences({ audio: { outputDevice: "alsa:hw:M4" } }).audio.outputDevice).toBe("alsa:hw:M4");
    for (const bad of ["", 3, {}, false, undefined]) {
      expect(sanitisePreferences({ audio: { outputDevice: bad } }).audio.outputDevice).toBeNull();
    }
  });
  it("enables rating imports for older settings and remembers explicit off choices", () => {
    expect(DEFAULT_PREFERENCES.usbExport).toMatchObject({ importButtonRatings: true });
    expect(sanitisePreferences({ usbExport: {} }).usbExport).toMatchObject({ importButtonRatings: true });
    expect(sanitisePreferences({ usbExport: { importButtonRatings: false } }).usbExport).toMatchObject({ importButtonRatings: false });
  });
  it("drops the old import-when-syncing choices: SYNC never imports (#199)", () => {
    const usb = sanitisePreferences({ usbExport: { importSettings: true, importHistory: true, importRatings: true } }).usbExport;
    expect(usb).not.toHaveProperty("importSettings");
    expect(usb).not.toHaveProperty("importHistory");
    expect(usb).not.toHaveProperty("importRatings");
  });
  it("enables rekordbox browse sync by default and remembers an explicit off choice", () => {
    expect(DEFAULT_PREFERENCES.rekordbox.syncBrowseSettings).toBe(true);
    expect(sanitisePreferences({ view: {} }).rekordbox.syncBrowseSettings).toBe(true);
    expect(sanitisePreferences({ rekordbox: { syncBrowseSettings: "no" } }).rekordbox.syncBrowseSettings).toBe(true);
    expect(sanitisePreferences({ rekordbox: { syncBrowseSettings: false } }).rekordbox.syncBrowseSettings).toBe(false);
  });
  it("protects a new library without making existing users read-only", () => {
    expect(DEFAULT_PREFERENCES.advanced.protectLibrary).toBe(true);
    expect(sanitisePreferences({ view: { tooltips: true } }).advanced.protectLibrary).toBe(false);
    expect(sanitisePreferences({ advanced: { protectLibrary: false } }).advanced.protectLibrary).toBe(false);
    expect(sanitisePreferences({ advanced: { protectLibrary: true } }).advanced.protectLibrary).toBe(true);
  });

  it("preserves the BPM-change visibility preference and enables it for older settings", () => {
    expect(sanitisePreferences({view: {showBpmChanges: false}}).view.showBpmChanges).toBe(false);
    expect(sanitisePreferences({view: {}}).view.showBpmChanges).toBe(true);
  });
  it("keeps a supported locale and rejects unknown ones", () => {
    expect(sanitisePreferences({ view: { locale: "ja" } }).view.locale).toBe("ja");
    expect(sanitisePreferences({ view: { locale: "xx" } }).view.locale).toBe("en");
  });
  it("keeps the LINK key-sort choice and defaults invalid or older settings to musical", () => {
    expect(sanitisePreferences({djSystem: {linkKeySort: "alphabetical"}}).djSystem.linkKeySort).toBe("alphabetical");
    expect(sanitisePreferences({djSystem: {linkKeySort: "invalid"}}).djSystem.linkKeySort).toBe("musical");
  });
  it("leaves automatic LINK joining off unless it is explicitly enabled", () => {
    expect(DEFAULT_PREFERENCES.djSystem.autoJoinLink).toBe(false);
    expect(sanitisePreferences({ djSystem: {} }).djSystem.autoJoinLink).toBe(false);
    expect(sanitisePreferences({ djSystem: { autoJoinLink: "yes" } }).djSystem.autoJoinLink).toBe(false);
    expect(sanitisePreferences({ djSystem: { autoJoinLink: true } }).djSystem.autoJoinLink).toBe(true);
  });
  it("leaves automatic analysis off unless it is explicitly enabled", () => {
    expect(DEFAULT_PREFERENCES.analysis.auto).toBe(false);
    expect(sanitisePreferences({ analysis: {} }).analysis.auto).toBe(false);
    expect(sanitisePreferences({ analysis: { auto: "yes" } }).analysis.auto).toBe(false);
    expect(sanitisePreferences({ analysis: { auto: true } }).analysis.auto).toBe(true);
  });

  it("leaves the first-beat memory cue off unless it is explicitly enabled", () => {
    expect(DEFAULT_PREFERENCES.analysis.firstBeatCue).toBe(false);
    expect(sanitisePreferences({ analysis: { firstBeatCue: "yes" } }).analysis.firstBeatCue).toBe(false);
    expect(sanitisePreferences({ analysis: { firstBeatCue: true } }).analysis.firstBeatCue).toBe(true);
    // "Write the value to the ID3 tag" is off until asked for, as rekordbox's
    // `writeKeyInTag` is (#204).
    expect(DEFAULT_PREFERENCES.analysis.writeKeyTag).toBe(false);
    expect(sanitisePreferences({ analysis: { writeKeyTag: "yes" } }).analysis.writeKeyTag).toBe(false);
    expect(sanitisePreferences({ analysis: { writeKeyTag: true } }).analysis.writeKeyTag).toBe(true);
  });

  it("leaves Add My Tag to the Comments off until it is switched on", () => {
    // rekordbox's `writeMytag` defaults to off (#605).
    expect(DEFAULT_PREFERENCES.advanced.myTagComments).toBe(false);
    expect(sanitisePreferences({ advanced: { myTagComments: "yes" } }).advanced.myTagComments).toBe(false);
    expect(sanitisePreferences({ advanced: { myTagComments: true } }).advanced.myTagComments).toBe(true);
  });

  it("analyses the key unless Analyze key is switched off", () => {
    expect(DEFAULT_PREFERENCES.analysis.detectKey).toBe(true);
    expect(sanitisePreferences({ analysis: { detectKey: "no" } }).analysis.detectKey).toBe(true);
    expect(sanitisePreferences({ analysis: { detectKey: false } }).analysis.detectKey).toBe(false);
  });

  it("offers rekordbox's ten BPM ranges with 70-180 the default", () => {
    expect(BPM_RANGES).toEqual(["70-180", "48-95", "58-115", "68-135", "78-155", "88-175", "98-195", "108-215", "118-235", "128-255"]);
    expect(DEFAULT_PREFERENCES.analysis.bpmRange).toBe("70-180");
    expect(sanitisePreferences({ analysis: { bpmRange: "118-236" } }).analysis.bpmRange).toBe("70-180");
    expect(sanitisePreferences({ analysis: { bpmRange: "108-215" } }).analysis.bpmRange).toBe("108-215");
    expect(bpmRangeLimits("128-255")).toEqual({ minBpm: 128, maxBpm: 255 });
  });

  it("keeps the browser key-sort choice and preserves the old display-based ordering", () => {
    expect(sanitisePreferences({view: {keySort: "musical"}}).view.keySort).toBe("musical");
    expect(sanitisePreferences({view: {keySort: "invalid"}}).view.keySort).toBe("auto");
    expect(sanitisePreferences({view: {keyDisplay: "classic"}}).view.keySort).toBe("auto");
    expect(sanitisePreferences({view: {keyDisplay: "alphanumeric"}}).view.keySort).toBe("auto");
  });
  it("gives the defaults for nothing, garbage, and a wrong shape", () => {
    expect(sanitisePreferences(undefined)).toEqual(DEFAULT_PREFERENCES);
    expect(sanitisePreferences("view")).toEqual(DEFAULT_PREFERENCES);
    expect(sanitisePreferences({ view: 3, advanced: [] })).toEqual(DEFAULT_PREFERENCES);
  });

  it("keeps each of the four waveform themes for the window, and only rekordbox's three for a device", () => {
    for (const choice of ["3band", "rgb", "blue", "theme"]) {
      expect(sanitisePreferences({ view: { waveformColor: choice } }).view.waveformColor).toBe(choice);
    }
    expect(sanitisePreferences({}).view.waveformColor).toBe("3band");
    expect(sanitisePreferences({ view: { waveformColor: "sepia" } }).view.waveformColor).toBe("3band");
    // A CDJ has no Theme palette: the setting written to a Stick stays one of its own.
    expect(sanitisePreferences({ djSystem: { waveformColor: "theme" } }).djSystem.waveformColor).toBe("3band");
  });

  it("keeps every valid choice and replaces each bad one on its own", () => {
    const stored = {
      view: {
        tooltips: false,
        browseFontSize: 4,
        browseLineSpace: 9,
        keyDisplay: "alphanumeric",
        keySort: "alphabetical",
        overviewWaveform: "full",
        explorer: "yes",
      },
      analysis: { auto: false },
      djSystem: { waveformColor: "rgb", subColumn: 5.5, categories: [{ id: 1 }], linkInterface: "" },
      advanced: {
        relocateFolders: ["/a", "", 3, "/b"],
        syncType: "bpm",
        quantizeBeat: "1/16",
        protectLibrary: true,
      },
    };
    const out = sanitisePreferences(stored);
    expect(out.view.tooltips).toBe(false);
    expect(out.view.browseFontSize).toBe(4);
    expect(out.view.browseLineSpace).toBe(BROWSE_SCALE_DEFAULT);
    expect(out.view.keyDisplay).toBe("alphanumeric");
    expect(out.view.keySort).toBe("alphabetical");
    expect(out.view.overviewWaveform).toBe("full");
    expect(out.view.explorer).toBe(true);
    expect(out.analysis.auto).toBe(false);
    expect(out.djSystem.waveformColor).toBe("rgb");
    expect(out.djSystem.subColumn).toBeNull();
    // A row that is not a row means the reference rows, not a broken list.
    expect(out.djSystem.categories).toBeNull();
    // An empty interface name is no choice: LINK picks for itself.
    expect(out.djSystem.linkInterface).toBeNull();
    expect(sanitisePreferences({ djSystem: { linkInterface: "en11" } }).djSystem.linkInterface).toBe("en11");
    expect(out.advanced.relocateFolders).toEqual(["/a", "/b"]);
    expect(out.advanced.syncType).toBe("bpm");
    expect(out.advanced.quantizeBeat).toBe("1/1");
    expect(out.advanced.protectLibrary).toBe(true);
  });

  it("searches Music, Video and Desktop by default, as rekordbox does", () => {
    // [OBS rekordbox 7.2.19 static] defaults 1, 1, 1 and 0 at 0x1037bbacc.
    const fresh = sanitisePreferences({ advanced: {} }).advanced;
    expect([fresh.relocateMusic, fresh.relocateVideo, fresh.relocateDesktop, fresh.relocateUserFolders])
      .toEqual([true, true, true, false]);
    const kept = sanitisePreferences({
      advanced: { relocateMusic: false, relocateVideo: false, relocateDesktop: false, relocateUserFolders: true },
    }).advanced;
    expect([kept.relocateMusic, kept.relocateVideo, kept.relocateDesktop, kept.relocateUserFolders])
      .toEqual([false, false, false, true]);
    // Folders saved before the box existed stay searched.
    expect(sanitisePreferences({ advanced: { relocateFolders: ["/a"] } }).advanced.relocateUserFolders).toBe(true);
  });

  it("drops the update settings an older store may still hold", () => {
    const kept = sanitisePreferences({ advanced: { checkUpdates: true, updateFrequency: "daily" } }).advanced;
    expect(kept).not.toHaveProperty("checkUpdates");
    expect(kept).not.toHaveProperty("updateFrequency");
  });

  it("copies a device playlist's missing tracks by default and keeps each other choice (#198)", () => {
    expect(DEFAULT_PREFERENCES.advanced.deviceImportMissing).toBe("copy");
    expect(sanitisePreferences({ advanced: {} }).advanced.deviceImportMissing).toBe("copy");
    expect(sanitisePreferences({ advanced: { deviceImportMissing: "skip" } }).advanced.deviceImportMissing).toBe("copy");
    for (const choice of ["copy", "ask", "matched", "cancel"] as const) {
      expect(sanitisePreferences({ advanced: { deviceImportMissing: choice } }).advanced.deviceImportMissing).toBe(choice);
    }
  });

  it("imports a same-named dropped folder as a copy by default and keeps each other choice (#355)", () => {
    expect(DEFAULT_PREFERENCES.advanced.folderDropClash).toBe("copy");
    expect(sanitisePreferences({ advanced: {} }).advanced.folderDropClash).toBe("copy");
    expect(sanitisePreferences({ advanced: { folderDropClash: "merge" } }).advanced.folderDropClash).toBe("copy");
    for (const choice of ["copy", "replace", "keep", "ask"] as const) {
      expect(sanitisePreferences({ advanced: { folderDropClash: choice } }).advanced.folderDropClash).toBe(choice);
    }
  });

  it("creates database folders on a blank drive unless the store plainly says not to", () => {
    expect(sanitisePreferences({ djSystem: {} }).djSystem.createDatabaseFolders).toBe(true);
    expect(sanitisePreferences({ djSystem: { createDatabaseFolders: "no" } }).djSystem.createDatabaseFolders).toBe(true);
    expect(sanitisePreferences({ djSystem: { createDatabaseFolders: false } }).djSystem.createDatabaseFolders).toBe(false);
  });

  it("keeps stored rows when every one is a row", () => {
    const rows = [{ id: 1, menuItem: 1, name: "GENRE", seq: 1, visible: true }];
    expect(sanitisePreferences({ djSystem: { categories: rows } }).djSystem.categories).toEqual(rows);
  });
});

describe("formatKey", () => {
  it("shows the library's name, or the Camelot code when asked", () => {
    expect(formatKey("Ebm", "classic")).toBe("Ebm");
    expect(formatKey("Ebm", "alphanumeric")).toBe("2A");
    expect(formatKey("C", "alphanumeric")).toBe("8B");
    expect(formatKey("12A", "classic")).toBe("Dbm");
    expect(formatKey("12A", "alphanumeric")).toBe("12A");
    // A key the wheel does not know is shown as it is, not hidden.
    expect(formatKey("Unknown", "alphanumeric")).toBe("Unknown");
    expect(formatKey("", "alphanumeric")).toBe("");
  });
});

describe("the sliders and the quantize value", () => {
  it("scale from the measured size in the middle", () => {
    expect(browseScale(BROWSE_SCALE_DEFAULT)).toBe(1);
    expect(browseScale(0)).toBeLessThan(1);
    expect(browseScale(4)).toBeGreaterThan(1);
    expect(browseScale(99)).toBe(1);
  });

  it("turn a beat value into a fraction of a beat", () => {
    expect(quantizeFraction("1/1")).toBe(1);
    expect(quantizeFraction("1/2")).toBe(0.5);
    expect(quantizeFraction("1/8")).toBe(0.125);
  });
});

 it("defaults Import to cues, history and ratings and preserves saved choices", () => {
  expect(sanitisePreferences({}).usbExport).toEqual({ deleteUnlistedMusic: false, maximumCompatibility: false, conversionFormat: "wav", verifyAllFiles: false, importButtonCues: true, importButtonHistory: true, importButtonSettings: false, importButtonRatings: true });
  expect(sanitisePreferences({ usbExport: { importButtonHistory: false, importButtonRatings: false } }).usbExport).toEqual({ deleteUnlistedMusic: false, maximumCompatibility: false, conversionFormat: "wav", verifyAllFiles: false, importButtonCues: true, importButtonHistory: false, importButtonSettings: false, importButtonRatings: false });
  expect(sanitisePreferences({ usbExport: { importButtonCues: false, importButtonSettings: true } }).usbExport).toMatchObject({ importButtonCues: false, importButtonHistory: true, importButtonSettings: true });
});

it("requires an explicit boolean to enable USB music cleanup", () => {
  expect(sanitisePreferences({ usbExport: { deleteUnlistedMusic: true } }).usbExport.deleteUnlistedMusic).toBe(true);
  expect(sanitisePreferences({ usbExport: { deleteUnlistedMusic: "true" } }).usbExport.deleteUnlistedMusic).toBe(false);
});

it("defaults compatibility conversion to off and WAV, and preserves AIFF and MP3 selections", () => {
  const defaults = sanitisePreferences({}).usbExport;
  expect(defaults.maximumCompatibility).toBe(false);
  expect(defaults.conversionFormat).toBe("wav");
  expect(sanitisePreferences({ usbExport: { maximumCompatibility: true, conversionFormat: "mp3" } }).usbExport)
    .toMatchObject({ maximumCompatibility: true, conversionFormat: "mp3" });
  expect(sanitisePreferences({ usbExport: { maximumCompatibility: true, conversionFormat: "aiff" } }).usbExport)
    .toMatchObject({ maximumCompatibility: true, conversionFormat: "aiff" });
  expect(sanitisePreferences({ usbExport: { maximumCompatibility: "yes", conversionFormat: "flac" } }).usbExport)
    .toMatchObject({ maximumCompatibility: false, conversionFormat: "wav", importButtonCues: true, importButtonHistory: true, importButtonSettings: false, importButtonRatings: true });
});

describe("browseListVars", () => {
  it("scales row height and font size from the Browse sliders", () => {
    const v = { browseFontSize: 4, browseLineSpace: 0, browseBold: true };
    const vars = browseListVars(v, 25);
    expect(vars["--s-row-height"]).toBe("20px");
    expect(vars["--f-size-ui"]).toBe("calc(1.3 * var(--f-size-ui-base))");
    expect(vars["--browse-weight"]).toBe(700);
  });
  it("is the measured size at the default stops", () => {
    const vars = browseListVars(
      { browseFontSize: BROWSE_SCALE_DEFAULT, browseLineSpace: BROWSE_SCALE_DEFAULT, browseBold: false },
      25,
    );
    expect(vars["--s-row-height"]).toBe("25px");
    expect(vars["--browse-weight"]).toBe(400);
  });
});

// Issue 540: rekordbox 7.2.19 keeps the Playback time setting in 1–60
// seconds, 60 by default, and reads anything else as the default
// (`SettingIF::getCurrentPlayedTime` @0x1015d43f8).
it("keeps the Playback time setting in rekordbox's 1 to 60 seconds, 60 by default", () => {
  expect(DEFAULT_PREFERENCES.advanced.playbackTime).toBe(60);
  expect(sanitisePreferences({}).advanced.playbackTime).toBe(60);
  expect(sanitisePreferences({ advanced: { playbackTime: 1 } }).advanced.playbackTime).toBe(1);
  expect(sanitisePreferences({ advanced: { playbackTime: 30 } }).advanced.playbackTime).toBe(30);
  for (const stored of [0, 61, 12.5, "30", null]) {
    expect(sanitisePreferences({ advanced: { playbackTime: stored } }).advanced.playbackTime).toBe(60);
  }
});

describe("the whole-interface scale", () => {
  it("starts at the measured size", () => {
    expect(DEFAULT_PREFERENCES.view.uiScale).toBe(UI_SCALE_DEFAULT);
    expect(uiScale(DEFAULT_PREFERENCES.view.uiScale)).toBe(1);
  });

  it("goes one stop smaller or larger at a time", () => {
    expect(UI_SCALES).toEqual([0.6, 0.7, 0.8, 0.9, 1, 1.1, 1.25, 1.5]);
    expect(uiScale(stepUiScale(4, "smaller"))).toBe(0.9);
    expect(uiScale(stepUiScale(4, "larger"))).toBe(1.1);
    expect(uiScale(stepUiScale(stepUiScale(4, "larger"), "larger"))).toBe(1.25);
  });

  it("stops at the smallest and the largest stop", () => {
    expect(uiScale(stepUiScale(0, "smaller"))).toBe(0.6);
    expect(uiScale(stepUiScale(7, "larger"))).toBe(1.5);
    let stop = 4;
    for (let press = 0; press < 20; press++) stop = stepUiScale(stop, "smaller");
    expect(uiScale(stop)).toBe(0.6);
    for (let press = 0; press < 20; press++) stop = stepUiScale(stop, "larger");
    expect(uiScale(stop)).toBe(1.5);
  });

  it("resets to the measured size from anywhere", () => {
    for (const stop of [0, 2, 4, 7]) expect(uiScale(stepUiScale(stop, "reset"))).toBe(1);
  });

  it("steps from the measured size when the stop it is given is not one", () => {
    expect(uiScale(stepUiScale(99, "smaller"))).toBe(0.9);
    expect(uiScale(stepUiScale(Number.NaN, "larger"))).toBe(1.1);
  });

  it("keeps a stored stop and reads anything else as the measured size", () => {
    expect(sanitisePreferences({}).view.uiScale).toBe(UI_SCALE_DEFAULT);
    expect(sanitisePreferences({ view: {} }).view.uiScale).toBe(UI_SCALE_DEFAULT);
    expect(sanitisePreferences({ view: { uiScale: 0 } }).view.uiScale).toBe(0);
    expect(sanitisePreferences({ view: { uiScale: 7 } }).view.uiScale).toBe(7);
    for (const stored of [-1, 8, 99, 2.5, 0.8, "3", null, Number.NaN]) {
      expect(sanitisePreferences({ view: { uiScale: stored } }).view.uiScale, String(stored)).toBe(UI_SCALE_DEFAULT);
    }
  });

  it("leaves the Browse font size and line space as they were stored", () => {
    const view = sanitisePreferences({ view: { uiScale: 1, browseFontSize: 4, browseLineSpace: 0 } }).view;
    expect(view).toMatchObject({ uiScale: 1, browseFontSize: 4, browseLineSpace: 0 });
  });
});

describe("the view shell switch", () => {
  it("is off unless it was turned on", () => {
    expect(DEFAULT_PREFERENCES.view.viewShell).toBe(false);
    expect(sanitisePreferences({}).view.viewShell).toBe(false);
    expect(sanitisePreferences({ view: {} }).view.viewShell).toBe(false);
  });

  it("keeps a stored on and reads anything that is not a yes or no as off", () => {
    expect(sanitisePreferences({ view: { viewShell: true } }).view.viewShell).toBe(true);
    for (const stored of ["true", 1, null]) {
      expect(sanitisePreferences({ view: { viewShell: stored } }).view.viewShell, String(stored)).toBe(false);
    }
  });
});

describe("the compact player's beat jump size", () => {
  it("starts on 16 beats and keeps a stored 4, 8, 16 or 32", () => {
    expect(DEFAULT_PREFERENCES.view.shellJumpBeats).toBe(16);
    for (const beats of [4, 8, 16, 32]) {
      expect(sanitisePreferences({ view: { shellJumpBeats: beats } }).view.shellJumpBeats).toBe(beats);
    }
  });

  it("reads any other stored value as the default", () => {
    for (const stored of [64, 0, "8", null]) {
      expect(sanitisePreferences({ view: { shellJumpBeats: stored } }).view.shellJumpBeats, String(stored)).toBe(16);
    }
  });
});
