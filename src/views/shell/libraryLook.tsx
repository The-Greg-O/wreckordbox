/**
 * How the Library view draws its rows (ticket #55): the look it hands the
 * list (`ListLook`). The list keeps its behaviour; this decides the density,
 * the Prep colour stripe, Energy's pips and the MyTags.
 *
 * docs/design/tokens.md: rows 24 px, headings 22 px, 12 px type, headings
 * 10.5 px uppercase with 0.08 em tracking, tabular numbers; Prep colour a
 * 3 px stripe at the row's left edge in rekordbox's own track colours;
 * Energy five pips coloured by level, unset pips in the hairline colour.
 */
import type { KeyboardEvent } from "react";

import { energyLevel, LIBRARY_MENU, LIBRARY_REQUIRED, prepColour, rowMyTags } from "@/lib/libraryColumns";
import { EXTRA_COLUMNS, type ColumnKey } from "@/lib/columns";
import type { RowDto } from "@/ipc/types";
import type { ListLook } from "@/views/browser/listLook";
import styles from "./LibraryList.module.css";

const PIPS = [1, 2, 3, 4, 5] as const;

/**
 * Energy as five pips. With `onRate` they are radio buttons, as the old
 * list's stars are: each focusable, pressed with the mouse, Enter or Space;
 * pressing the one that is set clears Energy. Without it (nothing may be
 * written) they are a picture of the level.
 */
export function EnergyPips({ rating, onRate }: { rating: number; onRate: ((stars: number) => void) | undefined }) {
  const level = energyLevel(rating);
  const label = level === 0 ? "Energy not set" : `Energy ${level} of 5`;
  if (!onRate) {
    return (
      <span className={styles.pips} data-level={level} role="img" aria-label={label}>
        {PIPS.map((pip) => <span key={pip} className={styles.pip} data-on={pip <= level || undefined} />)}
      </span>
    );
  }
  // The pip's own keys stay with it: the list and the deck hear Enter and
  // Space on the window, and must not also load or play a track. The group
  // is marked as the compact player is (OWN_ACTIVATION), so the shortcut
  // dispatcher ignores them too; the shell's capture-phase listener only
  // takes the view keys 1 to 4.
  const keepKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") e.stopPropagation();
  };
  return (
    <span className={styles.pips} data-level={level} role="radiogroup" aria-label={label} data-own-activation="">
      {PIPS.map((pip) => (
        <button
          key={pip}
          type="button"
          className={styles.pip}
          role="radio"
          aria-checked={pip === level}
          aria-label={`Energy ${pip}`}
          data-on={pip <= level || undefined}
          // A press on a pip is not a press on the row: it neither selects
          // the row nor, twice, loads the track.
          onMouseDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          onKeyDown={keepKey}
          onKeyUp={keepKey}
          onClick={(e) => {
            e.stopPropagation();
            onRate(pip === level ? 0 : pip);
          }}
        />
      ))}
    </span>
  );
}

function MyTags({ row }: { row: RowDto }) {
  const tags = rowMyTags(row);
  if (tags.length === 0) return null;
  return (
    <span className={styles.tags}>
      {tags.map((tag) => (
        <span key={tag.id} className={styles.tag} data-tag={tag.id} title={`${tag.category}: ${tag.name}`}>
          {tag.name}
        </span>
      ))}
    </span>
  );
}

function cell(key: ColumnKey, row: RowDto, rate: ((stars: number) => void) | undefined) {
  if (key === "rating") return <EnergyPips rating={row.rating} onRate={rate} />;
  if (key === "myTag") return <MyTags row={row} />;
  return undefined;
}

function edge(row: RowDto) {
  const colour = prepColour(row);
  if (colour === null) return null;
  return <span className={styles.prep} data-prep={colour} role="img" aria-label={`Prep colour: ${colour}`} />;
}

/** The fields the rows need: the shown columns' own, the colour for the stripe, and MyTags structured, not joined. */
function fetch(shown: readonly ColumnKey[]): readonly string[] {
  const own = EXTRA_COLUMNS.filter((key) => key !== "myTag" && key !== "color" && shown.includes(key));
  return [...own, "color", ...(shown.includes("myTag") ? ["myTags"] : [])];
}

export const LIBRARY_LOOK: ListLook = {
  className: styles.list ?? "",
  rowHeight: 24,
  headingHeight: 22,
  fetch,
  menu: { columns: LIBRARY_MENU, required: LIBRARY_REQUIRED },
  cell,
  edge,
};
