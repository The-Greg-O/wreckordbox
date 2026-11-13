/**
 * The Library view's second list: the target playlist (wbx-59,
 * `docs/design/screens/library/`). Title, BPM, key and Energy, with the
 * target's track count and length in its heading.
 *
 * It draws and reports; it holds no Library state. The target, its entries,
 * the selected entry and every edit belong to `useTargetList`, which hands
 * them in. A drop of tracks dragged from the main list adds them; a row
 * dragged within the list reorders it, by entry.
 *
 * The keyboard: the grid keeps the focus and names the selected row as its
 * active descendant, which is scrolled into view whenever it changes or
 * moves. The list's own commands (the cursor keys, Page Up / Down, Home /
 * End, Select All, as the DJ has bound them) move the second list's cursor
 * and stop there, as do Enter, Space and Delete, so neither the main list
 * nor the deck (which listen on the window) hears them, whether the list is
 * full, empty or still being read. A press in the list also takes the
 * browser's focus from the main list, so its focus-gated actions do not act
 * on a selection the DJ is not looking at.
 */
import { useEffect, useId, useRef, useState, type DragEvent, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import type { KeyDisplay } from "@/ipc/types";
import { setBrowserFocus } from "@/lib/browserFocus";
import { formatBpm } from "@/lib/format";
import { formatKey } from "@/lib/preferences";
import { describeChord, dispatch, OWN_ACTIVATION, type KeyOverrides, type Platform } from "@/lib/shortcuts";
import { dropInOrder, formatTargetLength, navigateTarget, TARGET_ACTIONS, type TargetCandidate } from "@/lib/targetList";
import { EnergyPips } from "./libraryLook";
import { LibraryKey } from "./LibraryKey";
import styles from "./TargetList.module.css";
import type { TargetRow } from "./useTargetRows";

export interface TargetListProps {
  /** The playlists that can be the target. */
  candidates: readonly TargetCandidate[];
  targetId: string | null;
  onChoose: (id: string | null) => void;
  /** The target's entries, in its order (as shown); `null` before the first read. */
  rows: readonly TargetRow[] | null;
  /** Why the latest read failed: the rows shown are stale. */
  failed: string | null;
  onRetry: () => void;
  /** The selected entry. */
  selected: string | null;
  onSelect: (entry: string | null) => void;
  /** A new order for the whole target, by entry, and the entry that moved. */
  onReorder: (order: string[], entry: string) => void;
  /** Tracks are being dragged from the main list. */
  dragging: boolean;
  onDropTracks: () => void;
  /** Why the last thing asked of the list was refused, if it was. */
  notice: string | null;
  keyDisplay: KeyDisplay;
  /** The loaded track's key, which the key cells light against. */
  loadedKey: string | null;
  platform: Platform;
  keyOverrides?: KeyOverrides | undefined;
}

const ROW_DRAG = "application/x-wreckordbox-target-row";

/** Keys the second list keeps from the window whatever they are bound to. */
const KEPT = new Set(["Enter", " ", "Delete", "Backspace"]);

export function TargetList({
  candidates, targetId, onChoose, rows, failed, onRetry, selected, onSelect, onReorder, dragging, onDropTracks, notice,
  keyDisplay, loadedKey, platform, keyOverrides,
}: TargetListProps) {
  const t = useTranslation();
  const prefix = useId();
  const rowId = (entry: string) => `${prefix}-entry-${entry.replace(/[^A-Za-z0-9_-]/g, "_")}`;
  const name = candidates.find((candidate) => candidate.id === targetId)?.name ?? null;
  const seconds = rows?.reduce((sum, row) => sum + row.row.durationSec, 0) ?? 0;
  const count = rows?.length ?? 0;
  const selectedIndex = selected === null || rows === null ? -1 : rows.findIndex((row) => row.entryId === selected);
  // The entry being dragged within the list, and the place a drop would land before.
  const carrying = useRef<number | null>(null);
  const [dropBefore, setDropBefore] = useState<number | null>(null);
  const grid = useRef<HTMLDivElement>(null);

  // The active row in view whenever it changes or moves.
  const order = rows?.map((row) => row.entryId).join(" ") ?? "";
  useEffect(() => {
    if (selected === null) return;
    const row = grid.current?.querySelector(`#${CSS.escape(rowId(selected))}`);
    row?.scrollIntoView?.({ block: "nearest" });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- rowId is a pure function of `prefix`
  }, [selected, order]);

  const keyOf = (action: string) => {
    const entry = TARGET_ACTIONS.find((candidate) => candidate.action === action);
    return entry ? describeChord(entry.chord, platform) : "";
  };

  const pageRows = () => {
    const height = grid.current?.clientHeight ?? 0;
    const first = grid.current?.querySelector<HTMLElement>('[role="row"][aria-rowindex]');
    const rowHeight = first?.offsetHeight ?? 24;
    return Math.max(1, Math.floor(height / Math.max(1, rowHeight)) - 1);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target as HTMLElement;
    // The target picker keeps its own keys; the main list ignores a field's.
    if (target.tagName === "SELECT") {
      if (event.key === "Enter" || event.key === " ") event.stopPropagation();
      return;
    }
    // A button in the list (Try again) presses itself.
    if (target.tagName === "BUTTON") {
      if (KEPT.has(event.key)) event.stopPropagation();
      return;
    }
    if (KEPT.has(event.key)) {
      event.stopPropagation();
      if (event.key !== "Enter") event.preventDefault();
      return;
    }
    const action = dispatch(event.nativeEvent, platform, null, keyOverrides);
    if (action === null) return;
    const next = navigateTarget(action, selectedIndex < 0 ? null : selectedIndex, count, pageRows());
    if (next === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    if (next !== null && rows !== null) onSelect(rows[next]!.entryId);
  };

  const placeFrom = (event: DragEvent<HTMLElement>, index: number) => {
    const box = event.currentTarget.getBoundingClientRect();
    return event.clientY > box.top + box.height / 2 ? index + 1 : index;
  };

  const onListDragOver = (event: DragEvent<HTMLElement>) => {
    if (carrying.current !== null) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      return;
    }
    if (!dragging || targetId === null) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  };
  const reorderTo = (from: number, before: number) => {
    if (rows === null) return;
    const moved = rows[from]!.entryId;
    const next = dropInOrder(rows.map((row) => row.entryId), from, before);
    if (next) onReorder(next, moved);
  };
  const onListDrop = (event: DragEvent<HTMLElement>) => {
    if (carrying.current !== null && rows !== null) {
      // Below every row: the end of the target.
      event.preventDefault();
      const from = carrying.current;
      carrying.current = null;
      setDropBefore(null);
      reorderTo(from, rows.length);
      return;
    }
    if (!dragging || targetId === null) return;
    event.preventDefault();
    onDropTracks();
  };

  return (
    <section
      className={styles.pane}
      aria-label={t("Second list")}
      {...{ [OWN_ACTIVATION]: "" }}
      data-testid="target-list"
      onKeyDown={onKeyDown}
      // A press here is not a press in the main list.
      onMouseDown={() => { setBrowserFocus(null); }}
    >
      <h2 className={styles.heading}>
        <span className={styles.mark} data-meaning-well="">▸ {t("target")}</span>
        <b className={styles.name}>{name ?? t("No target")}</b>
        {name !== null ? (
          <span className={styles.totals}>
            {t("{count} tracks", { count })} · {formatTargetLength(seconds)}
          </span>
        ) : null}
      </h2>
      <label className={styles.choose}>
        <span>{t("Target playlist")}</span>
        <select
          value={targetId ?? ""}
          onChange={(event) => { onChoose(event.currentTarget.value === "" ? null : event.currentTarget.value); }}
        >
          <option value="">{t("Choose a playlist…")}</option>
          {candidates.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>{candidate.path}</option>
          ))}
        </select>
      </label>
      {notice !== null ? <p className={styles.notice} role="status">{notice}</p> : null}
      {failed !== null ? (
        <p className={styles.failed} role="alert">
          {rows !== null ? t("Not current: the target could not be read again.") : t("The target could not be read.")}
          {" "}{failed}{" "}
          <button type="button" onClick={onRetry}>{t("Try again")}</button>
        </p>
      ) : null}
      <div
        ref={grid}
        className={styles.grid}
        role="grid"
        aria-label={t("Target")}
        aria-rowcount={count + 1}
        aria-busy={targetId !== null && rows === null && failed === null}
        aria-activedescendant={selected !== null && selectedIndex >= 0 ? rowId(selected) : undefined}
        data-stale={failed !== null || undefined}
        data-dropping={dragging || undefined}
        tabIndex={0}
        onDragOver={onListDragOver}
        onDrop={onListDrop}
      >
        <div role="row" className={styles.head}>
          <span role="columnheader" className={styles.title}>{t("Title")}</span>
          <span role="columnheader" className={styles.num}>{t("BPM")}</span>
          <span role="columnheader" className={styles.key}>{t("Key")}</span>
          <span role="columnheader" className={styles.energy}>{t("Energy")}</span>
        </div>
        {targetId === null ? (
          <p className={styles.empty}>{t("Choose a target playlist to add tracks to.")}</p>
        ) : rows !== null && rows.length === 0 ? (
          <p className={styles.empty}>{t("Empty. Drag tracks here, or select them and press {key}.", { key: keyOf("addToTarget") })}</p>
        ) : null}
        {rows?.map(({ entryId, row }, index) => (
          <div
            key={entryId}
            id={rowId(entryId)}
            role="row"
            aria-rowindex={index + 2}
            aria-selected={entryId === selected}
            className={styles.row}
            data-entry={entryId}
            data-drop={dropBefore === index ? "before" : dropBefore === index + 1 && index === rows.length - 1 ? "after" : undefined}
            draggable
            onClick={() => { onSelect(entryId); grid.current?.focus(); }}
            onDragStart={(event) => {
              carrying.current = index;
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData(ROW_DRAG, entryId);
              event.dataTransfer.setData("text/plain", row.id);
              event.stopPropagation();
            }}
            onDragEnd={() => { carrying.current = null; setDropBefore(null); }}
            onDragOver={(event) => {
              if (carrying.current === null) return;
              event.preventDefault();
              event.stopPropagation();
              event.dataTransfer.dropEffect = "move";
              setDropBefore(placeFrom(event, index));
            }}
            onDrop={(event) => {
              if (carrying.current === null) return;
              event.preventDefault();
              event.stopPropagation();
              const from = carrying.current;
              carrying.current = null;
              setDropBefore(null);
              reorderTo(from, placeFrom(event, index));
            }}
          >
            <span role="gridcell" className={styles.title} data-cell="title" title={row.title}>{row.title}</span>
            <span role="gridcell" className={styles.num} data-cell="bpm">{formatBpm(row.bpmX100)}</span>
            <span role="gridcell" className={styles.key} data-cell="key">
              <LibraryKey stored={row.key} loaded={loadedKey} text={formatKey(row.key, keyDisplay)} />
            </span>
            <span role="gridcell" className={styles.energy} data-cell="energy">
              <EnergyPips rating={row.rating} onRate={undefined} />
            </span>
          </div>
        ))}
      </div>
      <p className={styles.hint} aria-hidden>
        <kbd>{keyOf("addToTarget")}</kbd> {t("add selected")} · <kbd>{keyOf("moveTargetUp")}</kbd>/<kbd>{keyOf("moveTargetDown")}</kbd> {t("reorder")}
        {" "}· <kbd>{keyOf("swapTargetLists")}</kbd> {t("swap")} · <kbd>{keyOf("toggleTargetList")}</kbd> {t("hide")}
      </p>
    </section>
  );
}
