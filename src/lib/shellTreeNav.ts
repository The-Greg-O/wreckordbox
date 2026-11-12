/**
 * The shell's tree (ticket #52): where a key moves the focus, and which rows
 * are in view. Pure, so both are tested without a browser.
 */
import { isShellTreePlace, type ShellTreeRow } from "./shellTree";

/**
 * What a key asks of the tree from the focused row: move the focus, open or
 * close a folder, or nothing (`{}`: the key is still the tree's, so the
 * list and the deck do not hear it). Null for a key the tree does not take.
 */
export type KeyTarget = { focus: string } | { toggle: string; open: boolean } | Record<string, never>;

const NAVIGATION = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"]);

/** A heading or a Stick starts a group; a row's parent is never outside its group. */
function groupStart(rows: readonly ShellTreeRow[], index: number): number {
  for (let i = index; i >= 0; i -= 1) {
    const kind = rows[i]?.kind;
    if (kind === "heading" || kind === "stick") return i;
  }
  return 0;
}

/**
 * `isStop`: the rows the focus may land on. By default the places; the
 * shell's tree with actions adds what opens a menu without being a place
 * (the Playlists heading, a Stick library's label; wbx-45.4).
 */
export function keyTarget(
  rows: readonly ShellTreeRow[],
  from: string,
  key: string,
  isStop: (row: ShellTreeRow) => boolean = (row) => isShellTreePlace(row.kind),
): KeyTarget | null {
  if (!NAVIGATION.has(key)) return null;
  const index = rows.findIndex((row) => row.id === from);
  const row = rows[index];
  if (row === undefined) return {};
  const place = (i: number) => {
    const candidate = rows[i];
    return candidate !== undefined && isStop(candidate) ? candidate : undefined;
  };
  const step = (direction: 1 | -1) => {
    for (let i = index + direction; i >= 0 && i < rows.length; i += direction) {
      const found = place(i);
      if (found) return { focus: found.id };
    }
    return { focus: row.id };
  };
  const end = (direction: 1 | -1) => {
    for (let i = direction === 1 ? rows.length - 1 : 0; i >= 0 && i < rows.length; i -= direction) {
      const found = place(i);
      if (found) return { focus: found.id };
    }
    return {};
  };

  switch (key) {
    case "ArrowDown": return step(1);
    case "ArrowUp": return step(-1);
    case "Home": return end(-1);
    case "End": return end(1);
    case "ArrowLeft": {
      if (row.branch && row.open) return { toggle: row.id, open: false };
      const start = groupStart(rows, index);
      for (let i = index - 1; i >= start; i -= 1) {
        const found = place(i);
        if (found && found.kind !== "stick" && found.depth < row.depth) return { focus: found.id };
      }
      const head = rows[start];
      return head?.kind === "stick" && head.id !== row.id ? { focus: head.id } : {};
    }
    case "ArrowRight": {
      if (row.branch && !row.open) return { toggle: row.id, open: true };
      const next = rows[index + 1];
      if (row.kind === "stick") {
        for (let i = index + 1; i < rows.length && rows[i]?.kind !== "stick"; i += 1) {
          const found = place(i);
          if (found) return { focus: found.id };
        }
        return {};
      }
      if (row.branch && next !== undefined && isStop(next) && next.depth > row.depth) return { focus: next.id };
      return {};
    }
    default: return null;
  }
}

/** Heights of the tree's parts, in CSS pixels. */
export interface TreeSizes {
  row: number;
  heading: number;
  /** Between one group and the next. */
  gap: number;
  /** Above the first group and below the last. */
  pad: number;
}

export interface GroupBox {
  /** Where the group starts. */
  top: number;
  /** Where its first row starts, below its heading. */
  rowsTop: number;
  height: number;
}

/** Where each group sits, given whether it has a heading and how many rows it holds. */
export function treeLayout(groups: readonly (readonly [heading: boolean, rows: number])[], sizes: TreeSizes) {
  const boxes: GroupBox[] = [];
  let at = sizes.pad;
  groups.forEach(([heading, count], i) => {
    if (i > 0) at += sizes.gap;
    const rowsTop = at + (heading ? sizes.heading : 0);
    const height = rowsTop - at + count * sizes.row;
    boxes.push({ top: at, rowsTop, height });
    at += height;
  });
  return { groups: boxes, height: at + sizes.pad };
}

/**
 * The rows of a group to draw, `[first, end)`: those in the view from
 * `scroll` for `view` pixels, and `spare` more on either side.
 */
export function visibleRange(rowsTop: number, count: number, row: number, scroll: number, view: number, spare: number): [number, number] {
  const clamp = (n: number) => Math.max(0, Math.min(count, n));
  const first = clamp(Math.floor((scroll - rowsTop) / row) - spare);
  const end = clamp(Math.ceil((scroll + view - rowsTop) / row) + spare);
  return [first, Math.max(first, end)];
}
