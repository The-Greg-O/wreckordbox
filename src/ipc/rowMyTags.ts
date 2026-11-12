/**
 * A row page's MyTags (#55). Asked for as the extra field `myTags`, the
 * backend sends each of a track's MyTags with its id and category inside the
 * row's `extra`; this lifts them onto the row as `myTags`, typed, so nothing
 * downstream reads them from the extra fields or rebuilds them from the
 * joined names of the `myTag` column.
 */
import type { RowMyTag } from "./types";

function isTag(value: unknown): value is RowMyTag {
  if (typeof value !== "object" || value === null) return false;
  const tag = value as Record<string, unknown>;
  return typeof tag.id === "string" && typeof tag.name === "string" && typeof tag.category === "string";
}

export function liftMyTags<Row extends { id: string; extra?: Record<string, unknown> }>(rows: Row[]): (Row & { myTags?: RowMyTag[] })[] {
  return rows.map((row) => {
    const sent = row.extra?.myTags;
    if (sent === undefined) return row;
    const { myTags: _sent, ...extra } = row.extra ?? {};
    return { ...row, extra, myTags: Array.isArray(sent) ? sent.filter(isTag) : [] };
  });
}
