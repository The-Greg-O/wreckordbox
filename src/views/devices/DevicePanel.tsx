/**
 * The panel a selected device opens: six tabs across the top — General,
 * Category, Sort, Column, Color, My Settings — and the tab's body beneath.
 *
 * Drawn from the captures docs/screenshots 9.08.22 to 9.08.41 PM (rekordbox
 * 7.2.11, the TEST stick). The strip and every measurement in the tabs come
 * from them; My Settings has no capture and says so.
 *
 * The settings are the stick's own files, read when the panel opens (never
 * written then) and written on every change — there is no Apply button in the captures. Each
 * write resolves to what the stick now holds, which is what is shown, so a
 * write that did not take cannot leave the panel claiming it did.
 *
 * A stick whose last write did not finish is not read (that would settle
 * it): the panel says so, names the stick and what settling does, and
 * settles it only when the DJ presses that button (wbx-45.5).
 */
import { useStickWriteGateContext } from "@/views/shell/StickWriteGate";
import { useCallback, useEffect, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { Device, DeviceSettings, TreeNode } from "@/ipc/types";
import { errorMessage } from "@/lib/errorMessage";
import { isUnfinishedWrite, UNFINISHED_WRITE, unfinishedWriteWording } from "@/lib/stickRecovery";
import { usePreferences } from "@/store/usePreferences";
import { ColorTab } from "./ColorTab";
import { ColumnTab } from "./ColumnTab";
import styles from "./DevicePanel.module.css";
import { GeneralTab } from "./GeneralTab";
import { ListPairTab } from "./ListPairTab";
import { MySettingsTab } from "./MySettingsTab";

/** The six, in the order and with the wording of the capture [OBS]. */
const TABS = [
  { id: "general", label: "General" },
  { id: "category", label: "Category" },
  { id: "sort", label: "Sort" },
  { id: "column", label: "Column" },
  { id: "color", label: "Color" },
  { id: "mySettings", label: "My Settings" },
] as const;

export type DeviceTab = (typeof TABS)[number]["id"];

export interface DevicePanelProps {
  device: Device;
  /** Every playlist in the library, so one can be chosen to write. */
  playlists: readonly TreeNode[];
  /** Writes `playlistId` to the device. */
  onSync: (playlistId: string) => Promise<void>;
  /** Re-reads the connected volumes. */
  onRefresh: () => void;
  /** Something the stick refused; shown in the status bar. */
  onError?: (message: string) => void;
  /**
   * Settles the stick's unfinished write, the DJ's explicit action; resolves
   * once the stick has been read again. Without it the action is not offered.
   */
  onSettle?: (path: string) => Promise<void>;
  busy?: boolean;
}

export function DevicePanel({
  device,
  playlists,
  onSync,
  onRefresh,
  onError,
  onSettle,
  busy = false,
}: DevicePanelProps) {
  const [tab, setTab] = useState<DeviceTab>("general");
  const [settings, setSettings] = useState<DeviceSettings | null>(null);
  // The stick's last write did not finish; nothing is read until it is settled.
  const [unfinished, setUnfinished] = useState(false);
  const [settling, setSettling] = useState(false);
  // Bumped after settling, so the settings are read again.
  const [settled, setSettled] = useState(0);

  // Read when the device changes, and again after a sync: an export writes a
  // library, and with it the rows the Category, Sort and Color tabs edit.
  const exportStamp = device.export?.written ?? "";
  // A stick that holds an export but no DEVSETTING.DAT is shown with the DJ
  // System defaults. Opening the panel writes nothing to the stick
  // (wbx-45.5): the file is written with the first change the DJ makes here,
  // or by an export, both writes the DJ asked for.
  const stickDefaults = usePreferences().djSystem;
  const hasExport = device.export !== null;
  const gate = useStickWriteGateContext();
  useEffect(() => {
    let cancelled = false;
    setSettings(null);
    setUnfinished(false);
    void getBackend()
      .then(async (backend) => {
        const read = await backend.deviceSettings(device.path);
        if (read.hasDevSetting || !hasExport) return read;
        return {
          ...read,
          waveformColor: stickDefaults.waveformColor,
          waveformPosition: stickDefaults.waveformPosition,
          overviewWaveform: stickDefaults.overviewWaveform,
          keyDisplay: stickDefaults.keyDisplay,
        };
      })
      .then((read) => {
        if (!cancelled) setSettings(read);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        if (isUnfinishedWrite(e)) setUnfinished(true);
        else onError?.(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
    // The defaults change with every keystroke in Preferences; the stick is
    // read again only when the device or its export changes. A preference
    // changed while the panel is open is for the next stick.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [device.path, exportStamp, hasExport, onError, settled]);

  const settle = useCallback(() => {
    if (!onSettle) return;
    setSettling(true);
    onSettle(device.path)
      .then(() => setSettled((n) => n + 1))
      .catch((e: unknown) => onError?.(errorMessage(e)))
      .finally(() => setSettling(false));
  }, [device.path, onError, onSettle]);

  // Every change is a write to the Stick: staged, held for, then written,
  // and the panel shows what came back (wbx-45.4). Cancelled, it shows what
  // the Stick still holds.
  const save = useCallback(
    (next: DeviceSettings) => {
      if (gate === null) {
        onError?.("Settings on a Stick can only be changed from the main window.");
        return;
      }
      const before = settings;
      setSettings(next);
      void gate.stage({
        path: device.path,
        title: `Change the settings on ${device.name}`,
        lines: ["The Stick's own settings (what a player shows and sorts by) are written with this change."],
        plan: { kind: "saveSettings", settings: next },
        run: (approval) => getBackend()
          .then((backend) => backend.saveDeviceSettings(device.path, next, approval))
          .then(setSettings)
          .catch((e: unknown) => {
            onError?.(errorMessage(e));
          }),
      }).then((written) => {
        if (!written) setSettings(before);
      });
    },
    [device.path, device.name, gate, settings, onError],
  );

  if (unfinished) {
    const wording = unfinishedWriteWording(device.name);
    return (
      <section className={styles.panel} aria-label={`Device ${device.name}`}>
        <div className={styles.body}>
          <div role="group" aria-label={wording.label} className={styles.unfinished}>
            <p className={styles.unfinishedTitle}>{UNFINISHED_WRITE}</p>
            <p>{wording.what}</p>
            {onSettle ? (
              <button type="button" className={styles.unfinishedAction} onClick={settle} disabled={settling || busy}>
                {wording.action}
              </button>
            ) : null}
          </div>
        </div>
      </section>
    );
  }

  return (
    <section className={styles.panel} aria-label={`Device ${device.name}`}>
      <div className={styles.strip} role="tablist" aria-label="Device settings">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`device-tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`device-tabpanel-${t.id}`}
            className={styles.tab}
            data-on={tab === t.id || undefined}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div
        className={styles.body}
        role="tabpanel"
        id={`device-tabpanel-${tab}`}
        aria-labelledby={`device-tab-${tab}`}
      >
        {settings === null ? null : tab === "general" ? (
          <GeneralTab
            device={device}
            settings={settings}
            onChange={save}
            playlists={playlists}
            onSync={onSync}
            onRefresh={onRefresh}
            busy={busy}
          />
        ) : tab === "category" ? (
          <ListPairTab
            kind="category"
            slots={settings.categories}
            disabled={!settings.hasLibrarySettings}
            onChange={(categories) => save({ ...settings, categories })}
          />
        ) : tab === "sort" ? (
          <ListPairTab
            kind="sort"
            slots={settings.sorts}
            disabled={!settings.hasLibrarySettings}
            onChange={(sorts) => save({ ...settings, sorts })}
          />
        ) : tab === "column" ? (
          <ColumnTab settings={settings} onChange={save} />
        ) : tab === "color" ? (
          <ColorTab settings={settings} onChange={save} />
        ) : (
          <MySettingsTab />
        )}
      </div>
    </section>
  );
}
