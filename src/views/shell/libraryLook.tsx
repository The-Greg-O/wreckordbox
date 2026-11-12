/**
 * How the Library view draws its rows (ticket #55): the look it hands the
 * list (`ListLook`). The list keeps its behaviour; this decides the density,
 * the Prep colour stripe, Energy's pips and the MyTags.
 *
 * docs/design/tokens.md: rows 24 px, headings 22 px, 12 px type, headings
 * 10.5 px uppercase with 0.08 em tracking, tabular numbers; Prep colour a
 * 3 px stripe at the row's left edge in rekordbox's label colours; Energy
 * five pips coloured by level, unset pips in the hairline colour.
 */
import { energyLevel, LIBRARY_MENU, LIBRARY_REQUIRED, myTagNames, prepColour } from "@/lib/libraryColumns";
import type { RowDto } from "@/ipc/types";
import type { ColumnKey } from "@/lib/columns";
import type { ListLook } from "@/views/browser/listLook";
import styles from "./LibraryList.module.css";

const PIPS = [1, 2, 3, 4, 5] as const;

/** Five pips, the first `level` lit in that level's colour. Clicking one sets Energy, clicking the top lit one clears it. */
export function EnergyPips({ rating, onRate }: { rating: number; onRate: ((stars: number) => void) | undefined }) {
  const level = energyLevel(rating);
  return (
    <span
      className={styles.pips}
      data-level={level}
      role="img"
      aria-label={level === 0 ? "Energy not set" : `Energy ${level} of 5`}
    >
      {PIPS.map((pip) => (
        <span
          key={pip}
          className={styles.pip}
          data-pip={pip}
          data-on={pip <= level || undefined}
          // Rating from the row: the pip under the pointer, as the stars did.
          onMouseDown={onRate ? (e) => e.stopPropagation() : undefined}
          onDoubleClick={onRate ? (e) => e.stopPropagation() : undefined}
          onClick={onRate ? (e) => {
            e.stopPropagation();
            onRate(pip === level ? 0 : pip);
          } : undefined}
        />
      ))}
    </span>
  );
}

function MyTags({ value }: { value: unknown }) {
  const names = myTagNames(value);
  if (names.length === 0) return null;
  return (
    <span className={styles.tags}>
      {names.map((name) => <span key={name} className={styles.tag}>{name}</span>)}
    </span>
  );
}

function cell(key: ColumnKey, row: RowDto, rate: ((stars: number) => void) | undefined) {
  if (key === "rating") return <EnergyPips rating={row.rating} onRate={rate} />;
  if (key === "myTag") return <MyTags value={row.extra?.myTag} />;
  return undefined;
}

function edge(row: RowDto) {
  const colour = prepColour(row);
  if (colour === null) return null;
  return <span className={styles.prep} data-prep={colour} role="img" aria-label={`Prep colour: ${colour}`} />;
}

export const LIBRARY_LOOK: ListLook = {
  className: styles.list ?? "",
  rowHeight: 24,
  headingHeight: 22,
  fetch: ["color"],
  menu: { columns: LIBRARY_MENU, required: LIBRARY_REQUIRED },
  cell,
  edge,
};
