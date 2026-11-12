/**
 * The panel a selected device opens: six tabs across the top — General,
 * Category, Sort, Column, Color, My Settings — and the tab's body beneath.
 *
 * Drawn from the captures docs/screenshots 9.08.22 to 9.08.41 PM (rekordbox
 * 7.2.11, the TEST stick). The strip and every measurement in the tabs come
 * from them; My Settings has no capture and says so.
 *
 * The settings are the stick's own files, read when the panel opens and
 * written on every change — there is no Apply button in the captures. Each
 * write resolves to what the stick now holds, which is what is shown, so a
 * write that did not take cannot leave the panel claiming it did.
 */
import { useStickWriteGateContext } from "@/views/shell/StickWriteGate";
import { useCallback, useEffect, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { Device, DeviceSettings, TreeNode } from "@/ipc/types";
import { errorMessage } from "@/lib/errorMessage";
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
  busy?: boolean;
}

export function DevicePanel({
  device,
  playlists,
  onSync,
  onRefresh,
  onError,
  busy = false,
}: DevicePanelProps) {
  const [tab, setTab] = useState<DeviceTab>("general");
  const [settings, setSettings] = useState<DeviceSettings | null>(null);

  // Read when the device changes, and again after a sync: an export writes a
  // library, and with it the rows the Category, Sort and Color tabs edit.
  const exportStamp = device.export?.written ?? "";
  // Opening the panel only reads the stick. Every write to a Stick needs
  // the DJ's approval (wbx-45.4): the DJ System defaults rekordbox would
  // write on opening are not written unasked. (wbx-45.5 removes the same
  // automatic write; the two meet here.)
  const hasExport = device.export !== null;
  const gate = useStickWriteGateContext();
  useEffect(() => {
    let cancelled = false;
    setSettings(null);
    void getBackend()
      .then((backend) => backend.deviceSettings(device.path))
      .then((read) => {
        if (!cancelled) setSettings(read);
      })
      .catch((e: unknown) => {
        if (!cancelled) onError?.(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [device.path, exportStamp, hasExport, onError]);

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
