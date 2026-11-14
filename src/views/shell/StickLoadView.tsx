/**
 * The Stick view (wbx-71): the mounted Stick, whether it is a Gig stick or
 * a Spare stick and whether it is protected, when it was last loaded, its
 * space before and after, and what a load of the chosen playlists will do
 * (`docs/design/screens/stick`).
 *
 * Everything here reads through the Library service the window hosts
 * (`sticks.list`, `sticks.dry_run`): showing the view, choosing playlists
 * and the dry run write nothing to a Stick and never settle an unfinished
 * write. The load is staged through the window's Stick write gate like
 * every other Stick write (its plan, Stick Protection unlocked in place,
 * the hold: longer for a Gig stick), and the backend refuses it without
 * that hold's token or on a protected Stick (`stick_view.rs`).
 *
 * wbx-72: the hold is the view's own big button or `L`
 * (`StickHoldButton`), for exactly the plan shown; the gate approves that
 * plan without a dialog of its own. The load's state machine
 * (`holdToLoad.ts`) lives with the load in `src/store/stickLoads.ts`: while
 * it runs it fills this view with a plain progress bar (`Esc` shrinks it to
 * the status line), and it ends with what was written (or, failed, with
 * what was and was not) and an Eject. "Safe to remove" is said only once
 * that eject (the window's existing `eject_device`) has succeeded for that
 * Stick, never from a load's outcome (wbx-72 review).
 *
 * Reads are coalesced (one in flight, one pending) and every answer is
 * checked against what is asked now, so a late answer for another Stick or
 * other playlists is dropped; a failed read is shown, never the answer
 * before it.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { getBackend } from "@/ipc/client";
import type { Device, TreeNode } from "@/ipc/types";
import { coalescedRefresh } from "@/lib/coalescedRefresh";
import { OWN_ACTIVATION } from "@/lib/shortcuts";
import {
  chosenAfterToggle,
  formatBytes,
  lastLoadedLine,
  loadBlocked,
  outcomeLines,
  shortfallLine,
  spaceLine,
  summaryLines,
  type LoadOutcome,
  type LoadPlan,
  type LoadStick,
} from "@/lib/stickLoad";
import styles from "./StickLoadView.module.css";
import { useStickWriteGateContext } from "./StickWriteGate";
import { StickUnlock } from "./StickUnlock";
import { StickHoldButton } from "./StickHoldButton";
import { LoadProgressPanel, ShrunkLoad } from "./StickLoadProgress";
import {
  dismissStickLoad, dismissStickRemoval, ejectStick, machineOf, readKeptStickLoads, runStickLoad, stickIdentity, stickSeenWritten,
  takeWantedStick, useStickLoads, useStickRemovals, type StickLoad, type StickRemoval,
} from "@/store/stickLoads";

/** The playlists a load can include: the Library's playlists and folders, in tree order. */
export interface LoadablePlaylist {
  id: string;
  name: string;
  folder: boolean;
  depth: number;
}

/** The Library's playlists and folders (not intelligent playlists, Histories or a Stick's own). */
export function loadablePlaylists(tree: readonly TreeNode[]): LoadablePlaylist[] {
  // The tree is flat, in order, with depths; a Stick's own playlists are
  // `devicePlaylist` / `deviceFolder`, so `playlist` and `folder` are the Library's.
  return tree
    .filter((node) => node.kind === "folder" || node.kind === "playlist")
    .map((node) => ({ id: node.id, name: node.name, folder: node.kind === "folder", depth: node.depth }));
}

const CHOSEN_KEY = "rbl.stickLoad.chosen";

/** The DJ's last choice of playlists for a Stick, by its name (kept in this browser only). */
function rememberedChoice(name: string): string[] {
  try {
    const all = JSON.parse(localStorage.getItem(CHOSEN_KEY) ?? "{}") as Record<string, unknown>;
    const found = all[name];
    return Array.isArray(found) ? found.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}

function remember(name: string, chosen: readonly string[]): void {
  try {
    const all = JSON.parse(localStorage.getItem(CHOSEN_KEY) ?? "{}") as Record<string, unknown>;
    all[name] = chosen;
    localStorage.setItem(CHOSEN_KEY, JSON.stringify(all));
  } catch {
    // No storage: the choice lasts while the view is shown.
  }
}

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

type Read<T> = { state: "reading" } | { state: "read"; value: T } | { state: "failed"; error: string };

export function StickLoadView({ devices, selected, library, revision, panel, onEjected }: {
  /** The mounted volumes the window lists; the Sticks are read again when they change. */
  devices: readonly Device[];
  /** The Stick chosen in the tree, by its mount path. */
  selected: string | null;
  /** The Library's playlist tree. */
  library: readonly TreeNode[];
  /** Moves on when a Stick was written; the view reads again. */
  revision: number;
  /**
   * The selected Stick's own panel (its settings, and the named action that
   * settles an unfinished write: wbx-45.5), shown under the load.
   */
  panel?: ReactNode;
  /** A Stick was ejected from here: the window reads its mounted volumes again. */
  onEjected?: () => void;
}) {
  const gate = useStickWriteGateContext();
  const [sticks, setSticks] = useState<Read<LoadStick[]>>({ state: "reading" });
  const [picked, setPicked] = useState<string | null>(null);
  // A Stick chosen in the tree wins over one picked with the buttons before it.
  const [treeChoice, setTreeChoice] = useState(selected);
  if (treeChoice !== selected) {
    setTreeChoice(selected);
    setPicked(null);
  }
  const deviceKey = devices.map((d) => `${d.path}\u0000${d.volumeId}`).join("\u0001");

  // The Sticks: on show, when the mounted volumes change, when a Stick is
  // unlocked or locked anywhere, after a write.
  const stickReads = useRef<ReturnType<typeof coalescedRefresh> | null>(null);
  useEffect(() => {
    let live = true;
    const refresh = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        const found = await backend.stickViewSticks();
        if (live) setSticks({ state: "read", value: found });
      } catch (error) {
        if (live) setSticks({ state: "failed", error: message(error) });
      }
    });
    stickReads.current = refresh;
    refresh.request();
    const again = () => refresh.request();
    window.addEventListener("rbl:stick-protection", again);
    return () => {
      live = false;
      refresh.stop();
      window.removeEventListener("rbl:stick-protection", again);
    };
  }, [deviceKey, revision]);
  const readSticks = useCallback(() => stickReads.current?.request(), []);

  const list = sticks.state === "read" ? sticks.value : [];
  const stick = list.find((s) => s.path === (picked ?? selected)) ?? list.find((s) => s.path === selected) ?? list[0] ?? null;
  const device = stick === null ? undefined : devices.find((d) => d.path === stick.path);
  const total = stick === null ? null : stick.totalBytes ?? device?.totalBytes ?? null;
  // This Stick as mounted now: another volume at the same path is another Stick.
  const identity = stick === null ? null : stickIdentity(stick.path, device?.volumeId);
  // What the window last saw of its contents: a write moves it on.
  const written = `${revision}\u0000${device?.export?.written ?? ""}\u0000${device?.export?.tracks ?? ""}`;

  const playlists = useMemo(() => loadablePlaylists(library), [library]);
  const order = useMemo(() => playlists.map((p) => p.id), [playlists]);
  const [chosenBy, setChosenBy] = useState<Readonly<Record<string, string[]>>>({});
  const chosen = useMemo(() => {
    if (stick === null) return [];
    const ids = chosenBy[stick.path] ?? rememberedChoice(stick.name);
    const known = new Set(order);
    return ids.filter((id) => known.has(id));
  }, [stick, chosenBy, order]);
  const choose = (id: string) => {
    if (stick === null) return;
    const next = chosenAfterToggle(chosen, id, order);
    setChosenBy((current) => ({ ...current, [stick.path]: next }));
    remember(stick.name, next);
  };

  // The dry run of what is asked now; an answer for anything else is dropped.
  // Keyed by the Stick's identity and what was last written to it too, so a
  // Stick replaced at the same path, or written since, has no preview until
  // its own answer comes, and an answer for the Stick as it was is dropped.
  const asked = stick === null || chosen.length === 0 || stick.unfinishedWrite
    ? null
    : JSON.stringify([stick.path, chosen, identity, written]);
  const [plan, setPlan] = useState<{ key: string; read: Read<LoadPlan> } | null>(null);
  const live = useRef(asked);
  live.current = asked;
  const dryRuns = useRef<ReturnType<typeof coalescedRefresh> | null>(null);
  useEffect(() => {
    if (asked === null) return undefined;
    let mounted = true;
    const stops: (() => void)[] = [];
    const [path, ids] = JSON.parse(asked) as [string, string[], string, string];
    const refresh = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        const answer = await backend.stickViewDryRun(path, ids);
        if (mounted && live.current === asked) setPlan({ key: asked, read: { state: "read", value: answer } });
      } catch (error) {
        if (mounted && live.current === asked) setPlan({ key: asked, read: { state: "failed", error: message(error) } });
      }
    });
    dryRuns.current = refresh;
    refresh.request();
    void (async () => {
      const backend = await getBackend();
      if (!mounted) return;
      // An edit to the Library (anyone's) changes what a load would do.
      stops.push(backend.onLibraryChanged(() => refresh.request()));
    })();
    return () => {
      mounted = false;
      refresh.stop();
      for (const stop of stops) stop();
    };
  }, [asked]);
  const shown: Read<LoadPlan> | null = asked === null ? null : plan?.key === asked ? plan.read : { state: "reading" };

  const [panelOpen, setPanelOpen] = useState(false);
  // An unfinished write is settled in the Stick's panel: it opens itself then.
  const unfinished = stick?.unfinishedWrite === true;
  if (unfinished && !panelOpen) setPanelOpen(true);
  const [showChanges, setShowChanges] = useState(false);
  const [choosing, setChoosing] = useState(false);
  // With nothing chosen the choice is open, and stays open while the DJ ticks.
  if (stick !== null && chosen.length === 0 && !choosing) setChoosing(true);
  // Loads live above the view (`stickLoads`): this Stick's latest, and the others'.
  const allLoads = useStickLoads();
  // Reports the shell kept before this session (a load cut short by closing the window).
  useEffect(() => { void readKeptStickLoads(); }, []);
  const kept = allLoads.filter((l) => l.kept);
  // Removal readiness, apart from any load's outcome: only a succeeded eject says "safe to remove".
  const removals = useStickRemovals();
  // A Stick seen mounted and written again since its eject is no longer ready to remove.
  useEffect(() => {
    for (const d of devices) {
      stickSeenWritten(stickIdentity(d.path, d.volumeId), `${d.export?.written ?? ""}\u0000${d.export?.tracks ?? ""}`);
    }
  }, [devices]);
  const removalNotes = removals.length > 0 ? <Removals removals={removals} /> : null;
  const keptReports = kept.length > 0 || removalNotes !== null
    ? <>{removalNotes}{kept.length > 0 ? <KeptReports loads={kept} mounted={(path) => devices.some((d) => d.path === path)} /> : null}</>
    : null;
  // The status line asked for a load's Stick: shown, whichever was before.
  useEffect(() => {
    const wanted = takeWantedStick();
    if (wanted !== null) setPicked(wanted);
  }, [allLoads]);
  const current = identity === null ? null : [...allLoads].reverse().find((l) => l.identity === identity) ?? null;
  const others = allLoads.filter((l) => !l.kept && l.identity !== identity && l.state !== "confirming");
  const machine = machineOf(current);
  const loading = machine.state === "loading";
  const outcome = current?.state === "done" ? current.outcome : null;
  const loadError = current?.state === "failed" ? current.error : null;

  const ready = shown?.state === "read" ? shown.value : null;
  const blocked = stick === null ? null
    : stick.unfinishedWrite ? loadBlocked({ ...(ready ?? emptyPlan(stick)), stick }, chosen.length)
    : ready === null ? (chosen.length === 0 ? "Choose at least one playlist to load." : null)
    : loadBlocked(ready, chosen.length);

  // What a hold confirms: this Stick as mounted, its kind, and the plan shown.
  const binding = stick === null || identity === null || ready === null || ready.plan === null
    ? null
    : JSON.stringify([identity, stick.kind, ready.plan]);
  const load = (bound: string) => {
    if (stick === null || identity === null || ready === null || ready.plan === null || loading) return;
    // Held for exactly what is shown now, or nothing happens.
    if (bound !== binding) return;
    const servicePlan = ready.plan;
    const path = stick.path;
    const name = stick.name;
    const ids = [...chosen];
    const lines = summaryLines(ready).map((line) => `${line.count} ${line.text}`.trim());
    // The plan the DJ is shown is what the hold approves (its id), nothing else.
    const planned = {
      onStick: true, stickName: name, protected: stick.protected, spare: stick.kind === "spare", pending: false,
      audioEstimated: ready.changes.audioEstimated, tracksNew: 0, tracksChanged: 0, audioReused: 0, tracksRemoved: 0,
      tracksMissing: 0, trackChanges: [], analysisWritten: 0, playlists: [], netAdditionalBytes: 0, adding: null,
      plan: servicePlan,
    };
    void runStickLoad({ identity, path, name }, async (run) => {
      if (gate === null) throw new Error("Nothing was written: the Stick write confirmation is not available here.");
      return gate.stage({
        path,
        title: `Load ${ids.length} ${ids.length === 1 ? "playlist" : "playlists"} onto ${name}`,
        lines,
        plan: { kind: "sync", playlists: ids },
        planned,
        // Held on this view's own button or `L`, for exactly this plan.
        held: true,
        run: (approval) => run(async (operation) => (await getBackend()).stickViewLoad(path, ids, servicePlan, approval, operation)),
      });
    }).finally(() => {
      // Whoever is showing now reads again (a view gone stops its reads).
      stickReads.current?.request();
      dryRuns.current?.request();
    });
  };

  const removalOf = (id: string) => removals.find((r) => r.identity === id) ?? null;
  // The window's existing eject: it waits out and keeps out every writer to the Stick, then flushes and ejects.
  const eject = () => {
    if (stick === null || identity === null) return;
    void ejectStick({ identity, path: stick.path, name: stick.name }, `${device?.export?.written ?? ""}\u0000${device?.export?.tracks ?? ""}`,
      async (path) => { await (await getBackend()).ejectDevice(path); })
      .finally(() => { onEjected?.(); });
  };

  if (sticks.state === "reading") {
    return <div className={styles.view} data-testid="stick-view"><p className={styles.note} role="status">Looking for Sticks…</p></div>;
  }
  if (sticks.state === "failed") {
    return (
      <div className={styles.view} data-testid="stick-view" {...{ [OWN_ACTIVATION]: "" }}>
        <p className={styles.problem} role="alert">The Sticks could not be read: {sticks.error}</p>
        {keptReports}
        <div className={styles.buttons}><button type="button" onClick={readSticks}>Try again</button></div>
      </div>
    );
  }
  if (stick === null) {
    return (
      <div className={styles.view} data-testid="stick-view">
        <div className={styles.empty} data-testid="stick-none">
          <h2 className={styles.title}>No Stick is mounted</h2>
          <p className={styles.note}>Plug in a Stick to see what a load would put on it.</p>
          {keptReports}
        </div>
      </div>
    );
  }

  if (current !== null && machine.state === "loading" && !machine.shrunk) {
    return (
      <div className={styles.view} data-testid="stick-view" data-stick={stick.path} {...{ [OWN_ACTIVATION]: "" }}>
        <LoadProgressPanel load={current} progress={machine.progress} />
      </div>
    );
  }

  // Why the hold cannot be made now: what blocks the load, else the Stick still protected.
  const why = blocked ?? (stick.protected ? `Unlock ${stick.name} first.` : null);
  const space = ready?.space ?? null;
  const shortfall = space === null ? null : shortfallLine(space);
  const lines = ready === null ? [] : summaryLines(ready);
  const contents = stick.tracks === null ? "no library yet" : `${stick.tracks.toLocaleString("en")} tracks · ${stick.playlists ?? 0} playlists`;

  return (
    <div className={styles.view} data-testid="stick-view" data-stick={stick.path} {...{ [OWN_ACTIVATION]: "" }}>
      <section className={styles.main} aria-label={`Stick ${stick.name}`}>
        {keptReports}
        {list.length > 1 ? (
          <div className={styles.sticks} role="group" aria-label="Mounted Sticks">
            {list.map((s) => (
              <button key={s.path} type="button" aria-pressed={s.path === stick.path} onClick={() => { setPicked(s.path); }}>
                {s.name}
              </button>
            ))}
          </div>
        ) : null}
        <header className={styles.heading}>
          <h2 className={styles.title} data-testid="stick-name">{stick.name}</h2>
          <span className={styles.badge} data-kind={stick.kind} data-testid="stick-kind">{stick.kind === "spare" ? "Spare stick" : "Gig stick"}</span>
          <span className={styles.badge} data-protection={stick.protected ? "protected" : "unlocked"} data-testid="stick-protection">
            {stick.protected ? "Protected" : "Unlocked"}
          </span>
        </header>
        <p className={styles.meta} data-testid="stick-meta">{lastLoadedLine(stick.lastLoaded)} · {contents}</p>
        <div className={styles.unlock}>
          <StickUnlock
            path={stick.path}
            protection={{ protected: stick.protected, spare: stick.kind === "spare", name: stick.name, revision: 0 }}
            onChange={readSticks}
          />
        </div>

        {stick.unfinishedWrite ? (
          <p className={styles.problem} role="alert" data-testid="stick-unfinished">{blocked}</p>
        ) : null}

        <div className={styles.space} data-testid="stick-space">
          <h3 className={styles.label}>Space</h3>
          {space !== null ? (
            <>
              <p data-testid="stick-space-line">{spaceLine(space, total)}</p>
              <SpaceBar total={total} before={space.freeBefore} after={space.freeAfter} short={shortfall !== null} />
            </>
          ) : (
            <p data-testid="stick-space-line">
              {stick.freeBytes === null ? "Free space unknown" : `Free now ${formatBytes(stick.freeBytes)}${total === null ? "" : ` of ${formatBytes(total)}`}`}
            </p>
          )}
          {shortfall !== null ? <p className={styles.problem} role="alert" data-testid="stick-shortfall">{shortfall}</p> : null}
        </div>

        <div className={styles.summary}>
          <h3 className={styles.label}>This load will</h3>
          {chosen.length === 0 ? (
            <p className={styles.note} data-testid="stick-summary-none">Choose the playlists this load puts on {stick.name}.</p>
          ) : shown?.state === "failed" ? (
            <div>
              <p className={styles.problem} role="alert" data-testid="stick-dry-run-failed">What the load would do could not be worked out: {shown.error}</p>
              <div className={styles.buttons}><button type="button" onClick={() => dryRuns.current?.request()}>Try again</button></div>
            </div>
          ) : ready === null ? (
            stick.unfinishedWrite ? null : <p className={styles.note} role="status" data-testid="stick-summary-reading">Working out what the load will do…</p>
          ) : (
            <ul className={styles.lines} data-testid="stick-summary">
              {lines.map((line) => (
                <li key={line.kind} data-change={line.kind}>
                  <span className={styles.count}>{line.count}</span>
                  <span>{line.text}</span>
                </li>
              ))}
            </ul>
          )}
          {ready !== null && ready.plan === null ? (
            <p className={styles.problem} role="alert" data-testid="stick-not-loadable">{blocked}</p>
          ) : null}
          <div className={styles.buttons}>
            <button
              type="button"
              aria-expanded={showChanges}
              aria-controls="stick-changes"
              disabled={ready === null}
              onClick={() => { setShowChanges((open) => !open); }}
            >
              {showChanges ? "Hide the changes" : "Show every change"}
            </button>
            <button type="button" aria-expanded={choosing} aria-controls="stick-choose" onClick={() => { setChoosing((open) => !open); }}>
              Choose playlists{chosen.length > 0 ? ` (${chosen.length})` : ""}
            </button>
          </div>
          {choosing ? (
            <fieldset id="stick-choose" className={styles.choose} data-testid="stick-choose">
              <legend>Playlists this load puts on {stick.name}</legend>
              {playlists.length === 0 ? <p className={styles.note}>The Library has no playlists.</p> : null}
              {playlists.map((p) => (
                <label key={p.id} style={{ paddingLeft: `calc(${Math.max(0, p.depth - 1)} * var(--s-shell-space-5))` }}>
                  <input type="checkbox" checked={chosen.includes(p.id)} onChange={() => { choose(p.id); }} />
                  <span>{p.name}{p.folder ? " (folder: everything in it)" : ""}</span>
                </label>
              ))}
            </fieldset>
          ) : null}
          {showChanges && ready !== null ? <ChangeList plan={ready} /> : null}
        </div>

        {panel !== undefined && stick.path === selected ? (
          <div className={styles.panel}>
            <button type="button" aria-expanded={panelOpen} aria-controls="stick-panel" onClick={() => { setPanelOpen((open) => !open); }}>
              {panelOpen ? `Hide ${stick.name}'s settings and tools` : `${stick.name}'s settings and tools`}
            </button>
            {panelOpen ? <div id="stick-panel" data-testid="stick-panel">{panel}</div> : null}
          </div>
        ) : null}
      </section>

      <aside className={styles.side} aria-label="Load">
        {current !== null && machine.state === "loading" ? (
          <ShrunkLoad load={current} progress={machine.progress} />
        ) : current !== null && machine.state === "finished" ? (
          <section className={styles.finished} data-testid="stick-finished" role="status" aria-label="Load finished">
            <h3 className={styles.done}>The load of {current.name} finished</h3>
            <Outcome outcome={machine.outcome} />
            <EjectButton name={current.name} removal={removalOf(current.identity)} onEject={eject} />
            <button type="button" onClick={() => { dismissStickLoad(current.id); }}>Done</button>
          </section>
        ) : current !== null && machine.state === "failed" ? (
          <section className={styles.failed} data-testid="stick-failed" aria-label="Load failed">
            {loadError !== null ? <p className={styles.problem} role="alert" data-testid="stick-load-error">The load did not finish: {loadError}</p> : null}
            {outcome !== null ? <Outcome outcome={outcome} /> : null}
            {outcome === null ? <ul><li>Nothing was written</li></ul> : null}
            <EjectButton name={current.name} removal={removalOf(current.identity)} onEject={eject} />
            <button type="button" data-testid="stick-dismiss" onClick={() => { dismissStickLoad(current.id); }}>Dismiss this report</button>
          </section>
        ) : (
          <>
            <p className={styles.note}>
              Nothing is written until you hold the button.
            </p>
            <StickHoldButton
              name={stick.name}
              kind={stick.kind === "spare" ? "spare" : "gig"}
              enabled={why === null && ready !== null && ready.plan !== null && !loading}
              why={why}
              binding={binding}
              onHeld={load}
            />
            {why !== null ? <p id="stick-load-why" className={styles.note} data-testid="stick-load-why">{why}</p> : null}
          </>
        )}
        {others.length > 0 ? (
          <OtherLoads loads={others} mounted={(l) => devices.some((d) => stickIdentity(d.path, d.volumeId) === l.identity)} show={setPicked} />
        ) : null}
      </aside>
    </div>
  );
}

function emptyPlan(stick: LoadStick): LoadPlan {
  return {
    stick,
    playlists: [],
    changes: {
      counts: {
        tracksAdded: 0, tracksChanged: 0, audioChanged: 0, tracksRemoved: 0, tracksMissing: 0, audioReused: 0,
        playlistsCreated: 0, playlistsRewritten: 0, playlistsReordered: 0, playlistsRemoved: 0, nativeStems: 0,
      },
      tracks: { added: [], changed: [], audioChanged: [], removed: [], missing: [] },
      playlists: { created: [], rewritten: [], reordered: [], removed: [] },
      nativeStems: [], analysisWritten: 0, audioEstimated: true,
    },
    space: { freeBefore: null, neededBytes: 0, freeAfter: null, shortfallBytes: null, approximate: true },
    plan: null,
    notLoadable: null,
  };
}

/** Used space now and after the load, as a bar; the part the load adds in its own colour. */
function SpaceBar({ total, before, after, short }: { total: number | null; before: number | null; after: number | null; short: boolean }) {
  if (total === null || total <= 0 || before === null) return null;
  const usedBefore = Math.min(1, Math.max(0, (total - before) / total));
  const usedAfter = after === null ? (short ? 1 : usedBefore) : Math.min(1, Math.max(0, (total - after) / total));
  return (
    <div className={styles.bar} aria-hidden data-testid="stick-space-bar">
      <span className={styles.used} style={{ width: `${Math.min(usedBefore, usedAfter) * 100}%` }} />
      <span
        className={styles.adds}
        data-short={short || undefined}
        style={{ width: `${Math.abs(usedAfter - usedBefore) * 100}%` }}
      />
    </div>
  );
}

function Section({ title, items }: { title: string; items: readonly string[] }) {
  if (items.length === 0) return null;
  return (
    <section>
      <h4>{title} ({items.length})</h4>
      <ul>{items.map((item, at) => <li key={`${at}:${item}`}>{item}</li>)}</ul>
    </section>
  );
}

/** The dry run in full: every change, by kind. */
function ChangeList({ plan }: { plan: LoadPlan }) {
  const { tracks, playlists, nativeStems } = plan.changes;
  return (
    <div id="stick-changes" className={styles.changes} data-testid="stick-changes" role="region" aria-label="Every change">
      <Section title="Tracks added" items={tracks.added.map((t) => t.title)} />
      <Section title="Tracks updated" items={tracks.changed.map((t) => `${t.title}: ${t.fields.join(", ")}`)} />
      <Section title="Audio copied again" items={tracks.audioChanged.map((t) => t.title)} />
      <Section title="Native stems written" items={nativeStems.map((t) => t.title)} />
      <Section title="Tracks removed" items={tracks.removed.map((t) => t.title)} />
      <Section title="Not loaded: audio missing" items={tracks.missing.map((t) => t.title)} />
      <Section title="Playlists created" items={playlists.created} />
      <Section
        title="Playlists written again"
        items={playlists.rewritten.map((p) => {
          const parts = [
            p.added ? `+${p.added}` : null,
            p.removed ? `-${p.removed}` : null,
            p.reordered ? "reordered" : null,
          ].filter((part): part is string => part !== null);
          return parts.length > 0 ? `${p.name}: ${parts.join(", ")}` : p.name;
        })}
      />
      <Section title="Playlists taken off the Stick" items={playlists.removed} />
      {plan.changes.audioEstimated ? null : <p>The audio on the Stick could not be compared without writing to it, so the track lists are empty.</p>}
    </div>
  );
}

/** What the load did: what it wrote, what was already so, what it did not, every Native stems set, and what the Stick holds now. */
function Outcome({ outcome }: { outcome: LoadOutcome }) {
  const name = outcome.stick.name;
  const head = outcome.outcome === "loaded"
    ? `Loaded ${name}.`
    : outcome.outcome === "failed"
      ? `The load failed; ${name} holds what it held before.`
      : `The load did not finish everything on ${name}.`;
  return (
    <div className={styles.outcome} role="status" data-testid="stick-outcome" data-outcome={outcome.outcome}>
      <p className={outcome.outcome === "loaded" ? undefined : styles.problem}>
        {head}{outcome.error !== null ? ` ${outcome.error}` : ""}
      </p>
      <ul aria-label="What the load did">
        {outcomeLines(outcome).map((line) => <li key={line}>{line}</li>)}
      </ul>
      <Section title="Not written" items={[...outcome.notWritten.playlists, ...outcome.notWritten.tracks]} />
      {outcome.nativeStems.length > 0 ? (
        <section>
          <h4>Native stems ({outcome.nativeStems.length})</h4>
          <ul aria-label="Native stems">
            {outcome.nativeStems.map((stems, at) => (
              <li key={`${at}:${stems.title}`} data-stems={stems.outcome}>
                {stems.title}: {stems.outcome}{stems.reason !== null ? ` (${stems.reason})` : ""}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

/** The eject offered beside a finished or failed load. */
function EjectButton({ name, removal, onEject }: { name: string; removal: StickRemoval | null; onEject: () => void }) {
  return (
    <button type="button" className={styles.eject} data-testid="stick-eject" disabled={removal?.state === "ejecting"} onClick={onEject}>
      {removal?.state === "ejecting" ? `Ejecting ${name}…` : `Eject ${name}`}
    </button>
  );
}

/** What each eject asked from here came to: ready to remove only once it succeeded. */
function Removals({ removals }: { removals: readonly StickRemoval[] }) {
  return (
    <>
      {removals.map((removal) => (
        removal.state === "ejected" ? (
          <section key={removal.identity} className={styles.safeToRemove} role="status">
            <h3 className={styles.safe} data-testid="stick-safe-to-remove">{removal.name} is safe to remove</h3>
            <button type="button" onClick={() => { dismissStickRemoval(removal.identity); }}>OK</button>
          </section>
        ) : removal.state === "failed" ? (
          <section key={removal.identity} className={styles.failed} role="alert" data-testid="stick-eject-failed">
            <p className={styles.problem}>{removal.name} could not be ejected, so do not remove it yet: {removal.error}</p>
            <button type="button" onClick={() => { dismissStickRemoval(removal.identity); }}>Dismiss</button>
          </section>
        ) : null
      ))}
    </>
  );
}

/** Reports kept from before the window last closed, until the DJ dismisses each (also for a Stick not mounted). */
function KeptReports({ loads, mounted }: { loads: readonly StickLoad[]; mounted: (path: string) => boolean }) {
  return (
    <section className={styles.kept} data-testid="stick-kept-reports" aria-label="Loads reported since the window last closed">
      <h3 className={styles.label}>Earlier loads</h3>
      {loads.map((load) => (
        <div key={load.id} className={styles.keptReport} data-report={load.reportId}>
          <h4>{load.name}{mounted(load.path) ? "" : " (not mounted)"}</h4>
          {load.outcome !== null
            ? <Outcome outcome={load.outcome} />
            : (
              <div className={styles.outcome} role="status">
                <p className={styles.problem}>The load failed: {load.error ?? "no reason was given"}</p>
                <ul><li>Nothing was written</li></ul>
              </div>
            )}
          <button type="button" aria-label={`Dismiss the report for ${load.name}`} onClick={() => { dismissStickLoad(load.id); }}>
            Dismiss
          </button>
        </div>
      ))}
    </section>
  );
}

const LOAD_STATE: Record<StickLoad["state"], string> = { confirming: "confirming", running: "loading", done: "loaded", failed: "failed" };

/** Loads of other Sticks, so a result is never lost while another Stick is shown. */
function OtherLoads({ loads, mounted, show }: { loads: readonly StickLoad[]; mounted: (load: StickLoad) => boolean; show: (path: string) => void }) {
  return (
    <div className={styles.note} data-testid="stick-other-loads" role="status">
      <h4>Other Sticks</h4>
      <ul>
        {loads.map((load) => {
          const state = load.state === "done" && load.outcome !== null && load.outcome.outcome !== "loaded"
            ? load.outcome.outcome
            : LOAD_STATE[load.state];
          return (
            <li key={load.id}>
              {load.name}: {state}
              {mounted(load)
                ? <> <button type="button" onClick={() => { show(load.path); }}>Show {load.name}</button></>
                : " (no longer mounted)"}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
