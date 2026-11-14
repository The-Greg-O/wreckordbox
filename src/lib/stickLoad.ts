/**
 * The Stick view's reading of a load (wbx-71): the Library service's
 * `sticks.list`, `sticks.dry_run` and `sticks.load` answers (snake_case,
 * see `service-protocol.md` and `stick-load.md` in the notes) in the view's
 * terms, and the plain sentences the view shows. Pure: no backend, no DOM.
 */

/** A mounted Stick, as the service lists it. */
export interface LoadStick {
  path: string;
  name: string;
  /** `spare` only with the Spare stick marker; anything else is held for as a Gig stick. */
  kind: "gig" | "spare";
  /** Stick Protection: nothing is written until the DJ unlocks it. */
  protected: boolean;
  unfinishedWrite: boolean;
  /** The Stick's own record of wreckordbox's last load; null when it never has. */
  lastLoaded: string | null;
  freeBytes: number | null;
  totalBytes: number | null;
  tracks: number | null;
  playlists: number | null;
}

export interface TrackLine {
  id: string;
  title: string;
}

export interface LoadCounts {
  tracksAdded: number;
  tracksChanged: number;
  audioChanged: number;
  tracksRemoved: number;
  tracksMissing: number;
  audioReused: number;
  playlistsCreated: number;
  playlistsRewritten: number;
  playlistsReordered: number;
  playlistsRemoved: number;
  nativeStems: number;
}

export interface LoadChanges {
  counts: LoadCounts;
  tracks: {
    added: TrackLine[];
    changed: (TrackLine & { fields: string[] })[];
    audioChanged: TrackLine[];
    removed: TrackLine[];
    missing: TrackLine[];
  };
  playlists: {
    created: string[];
    rewritten: { name: string; added: number | null; removed: number | null; reordered: boolean | null }[];
    reordered: string[];
    removed: string[];
  };
  nativeStems: TrackLine[];
  analysisWritten: number;
  audioEstimated: boolean;
}

export interface LoadSpace {
  freeBefore: number | null;
  neededBytes: number;
  freeAfter: number | null;
  shortfallBytes: number | null;
  approximate: boolean;
}

/** What a load of the chosen playlists would do now (`sticks.dry_run`). */
export interface LoadPlan {
  stick: LoadStick;
  playlists: { id: string; name: string }[];
  changes: LoadChanges;
  space: LoadSpace;
  /** The plan id the load must carry; null when the service cannot plan a load for this Stick. */
  plan: string | null;
  notLoadable: string | null;
}

/** What a load did (`sticks.load`). */
export interface LoadOutcome {
  stick: LoadStick;
  outcome: "loaded" | "incomplete" | "failed";
  error: string | null;
  /** Changes the load made: of what it was to put on the Stick, what it wrote. */
  written: { tracks: string[]; playlists: string[] };
  /** Changes it set out to make and did not. */
  notWritten: { tracks: string[]; playlists: string[] };
  /** What it was to put there that was already as planned: neither written nor failed. */
  unchanged: { tracks: string[]; playlists: string[] };
  /** What the Stick holds now (its library, read back): not the same thing as what was written. */
  holds: { tracks: number | null; playlists: number | null };
  /** Every Native stems set: `written`, `skipped` or `failed`, and why. */
  nativeStems: { title: string; outcome: string; reason: string | null }[];
  /** The id the shell kept this report under (`stick_load_reports`), when it did. */
  reportId: string | null;
}

/** A load report the shell kept in the app's state folder, not yet dismissed. */
export interface KeptLoadReport {
  id: string;
  stick: { path: string; name: string };
  finished: string;
  /** The service's report; null when it gave none (refused or failed first). */
  outcome: LoadOutcome | null;
  /** Why, when there is no report. */
  error: string | null;
}

type Raw = Record<string, unknown>;

const record = (value: unknown): Raw => (value !== null && typeof value === "object" ? value as Raw : {});
const list = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);
const text = (value: unknown): string => (typeof value === "string" ? value : "");
const count = (value: unknown): number => (typeof value === "number" && Number.isFinite(value) ? value : 0);
const maybeNumber = (value: unknown): number | null => (typeof value === "number" && Number.isFinite(value) ? value : null);
const maybeText = (value: unknown): string | null => (typeof value === "string" ? value : null);
const maybeBool = (value: unknown): boolean | null => (typeof value === "boolean" ? value : null);
const strings = (value: unknown): string[] => list(value).map(text);
const track = (value: unknown): TrackLine => {
  const raw = record(value);
  return { id: text(raw.id), title: text(raw.title) };
};

function stick(value: unknown): LoadStick {
  const raw = record(value);
  return {
    path: text(raw.path),
    name: text(raw.name),
    kind: raw.kind === "spare" ? "spare" : "gig",
    protected: raw.protected !== false,
    unfinishedWrite: raw.unfinished_write === true,
    lastLoaded: maybeText(raw.last_loaded),
    freeBytes: maybeNumber(raw.free_bytes),
    totalBytes: maybeNumber(raw.total_bytes),
    tracks: maybeNumber(raw.tracks),
    playlists: maybeNumber(raw.playlists),
  };
}

export function fromServiceSticks(value: unknown): LoadStick[] {
  return list(record(value).sticks).map(stick);
}

function changes(value: unknown): LoadChanges {
  const raw = record(value);
  const c = record(raw.counts);
  const tracks = record(raw.tracks);
  const playlists = record(raw.playlists);
  return {
    counts: {
      tracksAdded: count(c.tracks_added),
      tracksChanged: count(c.tracks_changed),
      audioChanged: count(c.audio_changed),
      tracksRemoved: count(c.tracks_removed),
      tracksMissing: count(c.tracks_missing),
      audioReused: count(c.audio_reused),
      playlistsCreated: count(c.playlists_created),
      playlistsRewritten: count(c.playlists_rewritten),
      playlistsReordered: count(c.playlists_reordered),
      playlistsRemoved: count(c.playlists_removed),
      nativeStems: count(c.native_stems),
    },
    tracks: {
      added: list(tracks.added).map(track),
      changed: list(tracks.changed).map((entry) => ({ ...track(entry), fields: strings(record(entry).fields) })),
      audioChanged: list(tracks.audio_changed).map(track),
      removed: list(tracks.removed).map(track),
      missing: list(tracks.missing).map(track),
    },
    playlists: {
      created: strings(playlists.created),
      rewritten: list(playlists.rewritten).map((entry) => {
        const p = record(entry);
        return { name: text(p.name), added: maybeNumber(p.added), removed: maybeNumber(p.removed), reordered: maybeBool(p.reordered) };
      }),
      reordered: strings(playlists.reordered),
      removed: strings(playlists.removed),
    },
    nativeStems: list(raw.native_stems).map(track),
    analysisWritten: count(raw.analysis_written),
    audioEstimated: raw.audio_estimated !== false,
  };
}

export function fromServiceDryRun(value: unknown): LoadPlan {
  const raw = record(value);
  const space = record(raw.space);
  return {
    stick: stick(raw.stick),
    playlists: list(raw.playlists).map((entry) => ({ id: text(record(entry).id), name: text(record(entry).name) })),
    changes: changes(raw.changes),
    space: {
      freeBefore: maybeNumber(space.free_before),
      neededBytes: count(space.needed_bytes),
      freeAfter: maybeNumber(space.free_after),
      shortfallBytes: maybeNumber(space.shortfall_bytes),
      approximate: space.approximate === true,
    },
    plan: maybeText(raw.plan),
    notLoadable: maybeText(raw.not_loadable),
  };
}

function held(value: unknown): { tracks: string[]; playlists: string[] } {
  const raw = record(value);
  return { tracks: list(raw.tracks).map((entry) => track(entry).title), playlists: strings(raw.playlists) };
}

export function fromServiceLoad(value: unknown): LoadOutcome {
  const raw = record(value);
  const outcome = raw.outcome === "loaded" || raw.outcome === "incomplete" ? raw.outcome : "failed";
  return {
    stick: stick(raw.stick),
    outcome,
    error: raw.error === null || raw.error === undefined ? null : text(record(raw.error).message) || null,
    written: held(raw.written),
    notWritten: held(raw.not_written),
    unchanged: held(raw.unchanged),
    holds: { tracks: maybeNumber(record(raw.stick).tracks), playlists: maybeNumber(record(raw.stick).playlists) },
    reportId: maybeText(raw.report_id),
    nativeStems: list(raw.native_stems).map((entry) => {
      const s = record(entry);
      return { title: track(s.track).title, outcome: text(s.outcome), reason: maybeText(s.reason) };
    }),
  };
}

export function fromKeptReport(value: unknown): KeptLoadReport {
  const raw = record(value);
  const stick = record(raw.stick);
  const report = raw.report === null || raw.report === undefined ? null : fromServiceLoad(raw.report);
  const error = raw.error === null || raw.error === undefined ? null : text(record(raw.error).message) || null;
  return {
    id: text(raw.id),
    stick: { path: text(stick.path), name: text(stick.name) || text(stick.path) },
    finished: text(raw.finished),
    outcome: report === null ? null : { ...report, reportId: text(raw.id) },
    error,
  };
}

/** A size as the DJ reads it: decimal units, three figures. */
export function formatBytes(bytes: number): string {
  const units = [["GB", 1e9], ["MB", 1e6], ["kB", 1e3]] as const;
  for (const [unit, size] of units) {
    if (bytes >= size || unit === "kB") {
      const value = bytes / size;
      const shown = value >= 100 || unit !== "GB" ? Math.round(value).toString() : value.toFixed(1);
      return `${shown} ${unit}`;
    }
  }
  return `${bytes} B`;
}

/** "A", "A and B", "A, B and C". */
function names(items: readonly string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1) ?? ""}`;
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);

export type SummaryKind =
  | "added" | "stems" | "changed" | "audio" | "playlists" | "membership" | "removed" | "playlistsRemoved" | "missing" | "nothing";

export interface SummaryLine {
  kind: SummaryKind;
  /** The figure shown large beside the line ("+64", "-6"); empty for none. */
  count: string;
  text: string;
}

/** What the load will do, by kind of change, in plain counts. */
export function summaryLines(plan: LoadPlan): SummaryLine[] {
  const { counts, tracks, playlists } = plan.changes;
  const lines: SummaryLine[] = [];
  if (counts.tracksAdded > 0) {
    const across = plan.playlists.length > 0 ? `, across ${names(plan.playlists.map((p) => p.name))}` : "";
    lines.push({ kind: "added", count: `+${counts.tracksAdded}`, text: `${plural(counts.tracksAdded, "track", "tracks")} added${across}` });
  }
  if (counts.nativeStems > 0) {
    lines.push({ kind: "stems", count: `${counts.nativeStems}`, text: `${plural(counts.nativeStems, "track gets", "tracks get")} Native stems for the XDJ-AZ pads` });
  }
  if (counts.tracksChanged > 0) {
    const byField = new Map<string, number>();
    for (const changed of tracks.changed) for (const field of changed.fields) byField.set(field, (byField.get(field) ?? 0) + 1);
    const fields = [...byField].sort((a, b) => b[1] - a[1]).map(([field, n]) => ` · ${field} ${n}`).join("");
    lines.push({ kind: "changed", count: `${counts.tracksChanged}`, text: `${plural(counts.tracksChanged, "track", "tracks")} updated${fields}` });
  }
  if (counts.audioChanged > 0) {
    lines.push({ kind: "audio", count: `${counts.audioChanged}`, text: `${plural(counts.audioChanged, "track's audio", "tracks' audio")} copied again: it changed at the source` });
  }
  if (counts.playlistsCreated > 0) {
    const created = `${plural(counts.playlistsCreated, "playlist", "playlists")} created`;
    const reordered = counts.playlistsReordered > 0 ? ` · ${counts.playlistsReordered} reordered` : "";
    lines.push({ kind: "playlists", count: `${counts.playlistsCreated}`, text: `${created}${reordered}` });
  } else if (counts.playlistsReordered > 0) {
    lines.push({ kind: "playlists", count: `${counts.playlistsReordered}`, text: `${plural(counts.playlistsReordered, "playlist", "playlists")} reordered` });
  }
  // Playlists on the Stick already whose tracks change (a track already on
  // the Stick added to another chosen playlist, say): a change even when no
  // track is added or removed. One whose entries could not be compared
  // (an older export) is written again, so it counts too.
  const membership = playlists.rewritten.filter((p) => p.added === null || p.removed === null || p.added > 0 || p.removed > 0);
  if (membership.length > 0) {
    const shown = membership.slice(0, 3).map((p) => {
      if (p.added === null || p.removed === null) return `${p.name} (written again)`;
      const parts = [p.added > 0 ? `+${p.added}` : null, p.removed > 0 ? `−${p.removed}` : null].filter((x) => x !== null);
      return `${p.name} (${parts.join(" ")})`;
    });
    const more = membership.length > 3 ? ` and ${membership.length - 3} more` : "";
    lines.push({
      kind: "membership",
      count: `${membership.length}`,
      text: `${plural(membership.length, "playlist's tracks change", "playlists' tracks change")}: ${shown.join(", ")}${more}`,
    });
  }
  if (counts.tracksRemoved > 0) {
    lines.push({ kind: "removed", count: `-${counts.tracksRemoved}`, text: `${plural(counts.tracksRemoved, "track", "tracks")} removed` });
  }
  if (counts.playlistsRemoved > 0) {
    lines.push({
      kind: "playlistsRemoved",
      count: `${counts.playlistsRemoved}`,
      text: `${plural(counts.playlistsRemoved, "playlist", "playlists")} taken off the Stick: ${names(playlists.removed)}`,
    });
  }
  if (counts.tracksMissing > 0) {
    lines.push({ kind: "missing", count: `${counts.tracksMissing}`, text: `${plural(counts.tracksMissing, "track", "tracks")} not loaded: their audio file is missing` });
  }
  if (lines.length === 0) {
    lines.push({ kind: "nothing", count: "", text: "Nothing to change: the Stick already holds these playlists as they are." });
  }
  return lines;
}

const items = (tracks: number, playlists: number) =>
  [tracks > 0 ? `${tracks} ${plural(tracks, "track", "tracks")}` : null, playlists > 0 ? `${playlists} ${plural(playlists, "playlist", "playlists")}` : null]
    .filter((x) => x !== null)
    .join(", ");

/**
 * What a load did, as separate things: what it wrote, what was already as
 * planned, what it did not write, and what the Stick holds now.
 */
export function outcomeLines(outcome: LoadOutcome): string[] {
  const lines: string[] = [];
  const written = items(outcome.written.tracks.length, outcome.written.playlists.length);
  // "Needed" only for a load that succeeded with nothing to do.
  const none = outcome.outcome === "loaded" ? "Nothing needed writing" : "Nothing was written";
  lines.push(written === "" ? none : `Written: ${written}`);
  const unchanged = items(outcome.unchanged.tracks.length, outcome.unchanged.playlists.length);
  if (unchanged !== "") lines.push(`Already as planned: ${unchanged}`);
  const missed = items(outcome.notWritten.tracks.length, outcome.notWritten.playlists.length);
  if (missed !== "") lines.push(`Not written: ${missed}`);
  const { tracks, playlists } = outcome.holds;
  if (tracks !== null) {
    lines.push(`${outcome.stick.name} now holds ${tracks} ${plural(tracks, "track", "tracks")} in ${playlists ?? 0} ${plural(playlists ?? 0, "playlist", "playlists")}`);
  }
  return lines;
}

/** Free space now and after the load, against the Stick's size when it is known. */
export function spaceLine(space: LoadSpace, totalBytes: number | null): string | null {
  if (space.freeBefore === null) return null;
  const of = totalBytes !== null ? ` of ${formatBytes(totalBytes)}` : "";
  const after = space.freeAfter !== null ? ` · after this load ${formatBytes(space.freeAfter)}` : "";
  return `Free now ${formatBytes(space.freeBefore)}${of}${after}`;
}

/** By how much the load does not fit; null when it fits (or the free space is not known). */
export function shortfallLine(space: LoadSpace): string | null {
  if (space.shortfallBytes === null || space.shortfallBytes <= 0) return null;
  return `Not enough space: this load needs ${formatBytes(space.shortfallBytes)} more than the Stick has free.`;
}

/** Why the load cannot be started now, or null when it can. */
export function loadBlocked(plan: LoadPlan, chosen: number): string | null {
  if (plan.stick.unfinishedWrite) {
    return `A write to ${plan.stick.name} did not finish. Nothing is loaded until it is settled: open the Stick and choose "Finish or roll back the unfinished write on ${plan.stick.name}".`;
  }
  if (chosen === 0) return "Choose at least one playlist to load.";
  if (plan.plan === null) return `This Stick cannot be loaded: ${plan.notLoadable ?? "what the load would do could not be worked out."}`;
  return shortfallLine(plan.space);
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "Last loaded Nov 07" (with the year when it is not this one). */
export function lastLoadedLine(lastLoaded: string | null, now: Date = new Date()): string {
  if (lastLoaded === null) return "Never loaded by wreckordbox";
  const found = /^(\d{4})-(\d{2})-(\d{2})/.exec(lastLoaded);
  if (!found) return `Last loaded ${lastLoaded}`;
  const [, year, month, day] = found;
  const name = MONTHS[Number(month) - 1] ?? month;
  const thisYear = Number(year) === now.getUTCFullYear();
  return `Last loaded ${name} ${day}${thisYear ? "" : ` ${year}`}`;
}

/** The chosen playlists after one is ticked or unticked, in the tree's order. */
export function chosenAfterToggle(chosen: readonly string[], id: string, order: readonly string[]): string[] {
  const next = new Set(chosen);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return order.filter((entry) => next.has(entry));
}
