/**
 * The Library view's second list: the target playlist (wbx-59,
 * `docs/design/screens/library/`). Title, BPM, key and Energy, with the
 * target's track count and length in its heading.
 *
 * It draws and reports; it holds no Library state. The target, its rows,
 * the selected row and every edit belong to `useTargetList`, which hands
 * them in. A drop of tracks dragged from the main list adds them; a row
 * dragged within the list reorders it.
 *
 * Every key pressed inside the list stays inside it: the region owns its
 * activation (`data-own-activation`), and the keys it answers are stopped
 * before the list and the deck, which listen on the window, can hear them.
 */
import { useRef, useState, type DragEvent, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import type { RowDto } from "@/ipc/types";
import { formatBpm } from "@/lib/format";
import type { KeyDisplay } from "@/ipc/types";
import { formatKey } from "@/lib/preferences";
import { describeChord, OWN_ACTIVATION, type Platform } from "@/lib/shortcuts";
import { dropInOrder, formatTargetLength, TARGET_ACTIONS, type TargetCandidate } from "@/lib/targetList";
import { EnergyPips } from "./libraryLook";
import { LibraryKey } from "./LibraryKey";
import styles from "./TargetList.module.css";

export interface TargetListProps {
  /** The playlists that can be the target. */
  candidates: readonly TargetCandidate[];
  targetId: string | null;
  onChoose: (id: string | null) => void;
  /** The target's rows, in its order; `null` before the first read. */
  rows: readonly RowDto[] | null;
  /** Why the latest read failed: the rows shown are stale. */
  failed: string | null;
  onRetry: () => void;
  /** The selected row's place in the target. */
  selected: number | null;
  onSelect: (index: number | null) => void;
  /** A new order for the whole target, and where the moved row now is. */
  onReorder: (order: string[], index: number) => void;
  /** Tracks are being dragged from the main list. */
  dragging: boolean;
  onDropTracks: () => void;
  /** Why the last thing asked of the list was refused, if it was. */
  notice: string | null;
  keyDisplay: KeyDisplay;
  /** The loaded track's key, which the key cells light against. */
  loadedKey: string | null;
  platform: Platform;
}

const ROW_DRAG = "application/x-wreckordbox-target-row";

export function TargetList({
  candidates, targetId, onChoose, rows, failed, onRetry, selected, onSelect, onReorder, dragging, onDropTracks, notice,
  keyDisplay, loadedKey, platform,
}: TargetListProps) {
  const t = useTranslation();
  const name = candidates.find((candidate) => candidate.id === targetId)?.name ?? null;
  const seconds = rows?.reduce((sum, row) => sum + row.durationSec, 0) ?? 0;
  const count = rows?.length ?? 0;
  // The row being dragged within the list, and the row a drop would land before.
  const carrying = useRef<number | null>(null);
  const [dropBefore, setDropBefore] = useState<number | null>(null);
  const grid = useRef<HTMLDivElement>(null);

  const keyOf = (action: string) => {
    const entry = TARGET_ACTIONS.find((candidate) => candidate.action === action);
    return entry ? describeChord(entry.chord, platform) : "";
  };

  const onGridKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (rows === null || rows.length === 0) return;
    const last = rows.length - 1;
    const at = selected ?? -1;
    let next: number | null = null;
    if (event.altKey) return; // Alt + Up / Down is the reorder, heard on the window.
    if (event.key === "ArrowDown") next = Math.min(last, at + 1);
    else if (event.key === "ArrowUp") next = Math.max(0, at < 0 ? 0 : at - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = last;
    if (next === null) return;
    event.preventDefault();
    event.stopPropagation();
    onSelect(next);
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
  const onListDrop = (event: DragEvent<HTMLElement>) => {
    if (carrying.current !== null && rows !== null) {
      // Below every row: the end of the target.
      event.preventDefault();
      const from = carrying.current;
      carrying.current = null;
      setDropBefore(null);
      const order = dropInOrder(rows.map((row) => row.id), from, rows.length);
      if (order) onReorder(order, order.length - 1);
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
      onKeyDown={(event) => {
        // Enter and Space inside the list are its own, never the deck's or the main list's.
        if (event.key === "Enter" || event.key === " ") event.stopPropagation();
      }}
    >
      <h2 className={styles.heading}>
        <span className={styles.mark}>▸ {t("target")}</span>
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
        data-stale={failed !== null || undefined}
        data-dropping={dragging || undefined}
        tabIndex={0}
        onKeyDown={onGridKey}
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
        {rows?.map((row, index) => (
          <div
            // A playlist can hold one track twice: its place is the key.
            key={`${index}:${row.id}`}
            role="row"
            aria-rowindex={index + 2}
            aria-selected={index === selected}
            className={styles.row}
            data-drop={dropBefore === index ? "before" : dropBefore === index + 1 && index === rows.length - 1 ? "after" : undefined}
            draggable
            onClick={() => { onSelect(index); grid.current?.focus(); }}
            onDragStart={(event) => {
              carrying.current = index;
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData(ROW_DRAG, row.id);
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
              const before = placeFrom(event, index);
              const order = dropInOrder(rows.map((each) => each.id), from, before);
              if (order) onReorder(order, before > from ? before - 1 : before);
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
