import { ConfirmHost } from "@/components/ConfirmDialog";
import { removalQuestion } from "@/lib/collectionRemoval";
import type { DuplicateAnswer } from "@/components/DuplicateTracksDialog";
import type { FolderClashAnswer } from "@/lib/folderDrop";
import { JobQueue, slices, type JobView } from "@/lib/jobQueue";
import { importWithLedger, removeInSlices } from "@/lib/bulkJobs";
import { INITIAL_SYNC, deckStartedPlaying, isFollowing, pressMaster, toggleBeatSync, type SyncState } from "@/lib/syncMaster";
import { skippedTail } from "@/lib/importSkips";
import { useBackupProgress } from "@/store/useBackupProgress";
import { useExportProgress } from "@/store/useExportProgress";
import { isExportActive } from "@/lib/exportProgress";
import { reportStartupPaint } from "@/lib/startup";
import { useShowWindowWhenReady } from "@/lib/windowReady";
import { waveformKindOf } from "@/canvas";
import { useAutoJoinLink } from "@/store/useAutoJoinLink";
import { stickDevicesSeen } from "@/store/stickLoads";
import { useEventCallback } from "@/store/useEventCallback";
import { useCommands } from "@/store/useCommands";
/**
 * Export-mode shell.
 *
 * Composes the measured regions: top bar, library tree, track browser, status
 * bar. The preview player region is reserved but not yet implemented (it lands
 * with `rbl-audio` in Milestone 1).
 */
import type { TrackSearchField } from "@/lib/search";

import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useLibraryProtection } from "@/store/useLibraryProtection";
import { droppedFilePaths, getBackend, subscribeNativeFileDrops } from "@/ipc/client";
import type {
  Backend, DeckId, DeckTick, Device, ImportReport, LibraryProblem, LibrarySummary, RowDto, SortColumn, TrackField, TreeNode, ViewSpec,
} from "@/ipc/types";
import { TREE_ROOT } from "@/ipc/types";
import { TrackTable, type TrackDrag } from "@/views/browser/TrackTable";
import { clearWaveformPreviewCache } from "@/views/browser/WaveformPreview";
import { TreeView } from "@/views/tree/TreeView";
import { ConnectedTopBar } from "@/views/topbar/TopBar";
import { StatusBar } from "@/views/statusbar/StatusBar";
import { LinkDeckStrip } from "@/views/statusbar/LinkDeckStrip";
import styles from "./App.module.css";
import { locateLoadedTrack } from "@/lib/locateLoaded";
import { keyMap } from "@/lib/keyPresets";
import { detectPlatform, dispatch, isTyping, menuAccelerator, type Action } from "@/lib/shortcuts";
import { hasCommand, runCommand } from "@/lib/commands";
import { browserFocus, followBrowserFocus, onBrowserFocus } from "@/lib/browserFocus";
import { menuBarEnabled, menuBarSide, menuBarState, onMenuBarState } from "@/lib/menuBar";
import { hasEditHistory, runEditHistory, setLibraryEditHistory } from "@/lib/editHistory";
import { transposeKey } from "@/lib/camelot";
import { stepMasterGain, toggleMute } from "@/lib/volume";
import { clampWidth, TREE_BOUNDS } from "@/lib/splitter";
import { exportSummary } from "@/lib/exportSummary";
import { askDevicePlaylistDrop } from "@/lib/devicePlaylistDrop";
import { deviceId, devicePath, renamedDevice } from "@/lib/devices";
import { DEVICE_ASKS, deviceNodeId, deviceParentFor, devicePlaylistsOf, isDeviceLibraryKind, LIBRARY_NAMES, parseDeviceNodeId } from "@/lib/deviceLibrary";
import { useDeviceLibraries } from "@/store/useDeviceLibraries";
import { refusal, resolveMenu } from "@/lib/menu";
import { nextSort, specForNode, type SortState } from "@/lib/viewSpec";
import {
  DEFAULT_SUB_TREE_WIDTH, DEFAULT_SUB_WIDTH, loadSession, saveSession, SEEDED_NODES, SEEDED_ROWS,
  type TrafficLightSource,
  type ArtworkMode,
} from "@/lib/session";
import { startWindowDrag, toggleWindowMaximise } from "@/lib/windowDrag";
import { AppCost } from "@/views/topbar/AppCost";
import { useAudioOutput } from "@/store/useAudioOutput";
import { useLimiter } from "@/store/useLimiter";
import { onPreviewError, stopPreview } from "@/store/usePreview";
import { MasterOutputProvider, MasterOutputConnection, useMasterControls, useMasterDisplay } from "@/store/MasterOutput";
import { asLayout, deckCount, isFullDeck, type PlayerLayout } from "@/lib/layout";
import { DEFAULT_SHELL_VIEW, type ShellView } from "@/lib/shellViews";
import { useLibraryColumns } from "@/store/useLibraryColumns";
import { useShellWidth } from "@/store/useShellWidth";
import { libraryWidthOf } from "@/lib/libraryFit";
import { libraryLookFor } from "@/views/shell/libraryFitLook";
import { SHELL_PLAYER_LAYOUT, ViewShell } from "@/views/shell/ViewShell";
import { useShellTree } from "@/views/shell/useShellTree";
import { WaveformThemeChooser } from "@/views/shell/WaveformThemeChooser";
import { breadcrumbOf } from "@/lib/shellTreeOverlay";
import { crumbTally } from "@/lib/crumbTally";
import { useTargetList } from "@/views/shell/useTargetList";
import { IntakeOnDemand, preloadShellViews, ReviewOnDemand, StickLoadOnDemand } from "@/views/shell/shellOnDemand";
import { useReviewCount } from "@/views/shell/reviewQueueStore";
import { useIntakeCounts } from "@/views/shell/useIntakeCounts";
import { useStickOutlines } from "@/store/useStickOutlines";
import { useStickProtection } from "@/store/useStickProtection";
import { StickProtectionDialog } from "@/views/shell/StickProtectionDialog";
import { StickWriteGateProvider, useStickWriteGate } from "@/views/shell/StickWriteGate";
import { shellOpensNode, stickPlaylistsUnder } from "@/lib/stickWrites";
import { FIELD_LABEL } from "@/views/info/fieldLabel";
import { SubBrowser } from "@/views/subbrowser/SubBrowser";
import { RightRail } from "@/views/browser/RightRail";
import { MyTagPanel } from "@/views/browser/MyTagPanel";
import { columnContextFor, useColumns, type ColumnContext } from "@/store/useColumns";
import { useExplorer } from "@/store/useExplorer";
import { hasLooseId, importLoose, isLooseId } from "@/lib/explorer";
import { childrenOf, parentFor, playlistPaths, withSources } from "@/lib/tree";
import { DETAIL_BARS } from "@/lib/player";
import type { Deck as SyncDeck } from "@/lib/sync";
import { LayoutDualIcon } from "@/components/icons";
import { Player } from "@/views/player/Player";
import { MixerStrip } from "@/views/player/MixerStrip";
import { DualZoom } from "@/views/player/DualDeck";
import type { PreferencesTarget } from "@/views/settings/Preferences";
import { PreferencesProvider, usePreferencesStore } from "@/store/usePreferences";
import { formatKey, stepUiScale, type PreferencePane } from "@/lib/preferences";
import { useEffectiveWaveform } from "@/lib/waveTheme";
import { answer, deckNumber, setPlaying, whenLoaded, withSetting, type ScriptHandler } from "@/lib/scripting";
import { continuesFrom, playedToEnd, skipRowIndex, type ListPosition } from "@/lib/continuousPlay";
import { useAnalysis } from "@/store/useAnalysis";
import type { AnalysisChoice } from "@/views/analysis/AnalysisDialog";
import { autoAnalysisOffer, takeRemainingPages } from "@/lib/autoAnalysis";
import type { LibraryQuestion } from "@/views/library/NewLibraryDialog";
import type { QueueItem } from "@/lib/queue";
import { TrackFilter } from "@/views/browser/TrackFilter";
import { TrackFilterBar } from "@/views/shell/TrackFilterBar";
import { useShellTrackFilter } from "@/views/shell/useShellTrackFilter";
import { useSaveFilterPlaylist } from "@/views/shell/SaveFilterPlaylist";
import { hasPicks } from "@/lib/shellTrackFilter";
import { EMPTY_FILTER, toSpecFilter, type FilterState } from "@/lib/trackFilter";
import type { AnalysisResult, DeviceFormat, DeviceImportMissing, DevicePlaylistEdit, DevicePlaylistImportPlan, FilterValues, LinkPeerSeen, LinkStatus, ListTotals, RelocateSearch, SaveFilterRequest, SmartRule, StickWriteOperation, TrackLookups, UnanalysedTracks } from "@/ipc/types";
import { statusBarListText } from "@/lib/format";
import { StatusLine } from "@/views/shell/StatusLine";
import { autoRelocated, missingAmong, relocateSteps, relocateTracks } from "@/lib/relocate";
import { useTooltip } from "@/store/usePreferences";
import { useTranslation } from "@/i18n";
import { CommandPaletteButton, CommandPaletteHost } from "@/views/palette/CommandPalette";
import { nativeMenuLabels } from "@/lib/nativeMenu";

/** The commands the shell itself registers, and so the keys it runs. */
const SHELL_COMMANDS: ReadonlySet<Action> = new Set<Action>([
  "volumeUp", "volumeDown", "mute", "focusSearch", "clearSearch", "analyseSelection", "sortItems",
  "uiScaleSmaller", "uiScaleLarger", "uiScaleReset",
]);

/**
 * The metadata fields a row already carries, so an edit to one can be shown
 * before the backend has answered. The others live only in the information
 * panel's record, which is re-read anyway.
 */
const ROW_FIELDS: ReadonlySet<TrackField> = new Set<TrackField>([
  "title", "artist", "album", "genre", "label", "key",
]);

const DuplicateTracksDialog = lazy(() => import("@/components/DuplicateTracksDialog").then(m => ({ default: m.DuplicateTracksDialog })));
const FolderClashDialog = lazy(() => import("@/components/FolderClashDialog").then(m => ({ default: m.FolderClashDialog })));
const DeviceImportCopyDialog = lazy(() => import("@/components/DeviceImportCopyDialog").then(m => ({ default: m.DeviceImportCopyDialog })));
const AnalysisDialog = lazy(() => import("@/views/analysis/AnalysisDialog").then(m => ({ default: m.AnalysisDialog })));
const NewLibraryDialog = lazy(() => import("@/views/library/NewLibraryDialog").then(m => ({ default: m.NewLibraryDialog })));
/** No Sticks: what the shell's tree reads while the shell is off. */
const DevicePanel = lazy(() => import("@/views/devices/DevicePanel").then(m => ({ default: m.DevicePanel })));
const InfoPanel = lazy(() => import("@/views/info/InfoPanel").then(m => ({ default: m.InfoPanel })));
const LogView = lazy(() => import("@/views/log/LogView").then(m => ({ default: m.LogView })));
const Preferences = lazy(() => import("@/views/settings/Preferences").then(m => ({ default: m.Preferences })));
const SyncManager = lazy(() => import("@/views/sync/SyncManager").then(m => ({ default: m.SyncManager })));
const SmartPlaylistEditor = lazy(() => import("@/views/tree/SmartPlaylistEditor").then(m => ({ default: m.SmartPlaylistEditor })));
const MissingFileManager = lazy(() => import("@/views/library/MissingFileManager").then(m => ({ default: m.MissingFileManager })));

/**
 * How many tracks one backend call handles in a long operation. Each call is
 * one database write, so a slice is the unit the status bar's progress moves
 * in and the longest a Stop waits.
 */
const ADD_SLICE = 100;

function ConnectedPreferences(props: Omit<React.ComponentProps<typeof Preferences>, "reduction" | "vu" | "peakLeft" | "peakRight">) {
  const master = useMasterDisplay();
  return <Preferences {...props} reduction={master.reduction} vu={master.vu}
    peakLeft={master.peakLeft} peakRight={master.peakRight} />;
}

function useClock(): string {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    // Tick on the minute boundary rather than polling: idle CPU budget is 0.5%.
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      timer = setTimeout(() => {
        setNow(new Date());
        schedule();
      }, 60_000 - (Date.now() % 60_000));
    };
    schedule();
    return () => clearTimeout(timer);
  }, []);
  return `${now.getHours() % 12 || 12}:${String(now.getMinutes()).padStart(2, "0")}`;
}

export function App() {
  return <MasterOutputProvider><AppBody /></MasterOutputProvider>;
}

function AppBody() {
  const t = useTranslation();
  useShowWindowWhenReady();
  useEffect(() => { reportStartupPaint("shell-painted"); }, []);
  useEffect(() => {
    void getBackend().then((backend) => backend.setMenuLabels(nativeMenuLabels(t))).catch(() => undefined);
  }, [t]);
  useEffect(() => {
    // Native file drags can land anywhere, including outside a drop target.
    // Cancel WebKit's file navigation without stopping playlist/deck handlers.
    const preventFileNavigation = (event: DragEvent) => {
      if (event.dataTransfer?.types.includes("Files") || event.dataTransfer?.files.length) {
        event.preventDefault();
      }
    };
    // Accept file drags first so WebKit dispatches a DOM drop instead of
    // falling back to native navigation before a drop handler can run.
    document.addEventListener("dragover", preventFileNavigation, true);
    document.addEventListener("drop", preventFileNavigation, true);
    return () => {
      document.removeEventListener("dragover", preventFileNavigation, true);
      document.removeEventListener("drop", preventFileNavigation, true);
    };
  }, []);
  // Read once, synchronously, so the first render is already the layout the
  // window closed with rather than the default that then jumps.
  const [restored] = useState(loadSession);

  const [tree, setTree] = useState<readonly TreeNode[]>(restored.tree);
  const [treeExpansion, setTreeExpansion] = useState(restored.treeExpansion);
  const [editHistory, setEditHistory] = useState({
    canUndo: false, canRedo: false, undoLabel: null as string | null, redoLabel: null as string | null,
  });
  // Connected volumes. Asked for, never polled: a 1 Hz scan of every mount
  // point is exactly the kind of idle work the budgets forbid.
  const [devices, setDevices] = useState<readonly Device[]>([]);
  // A Stick ejected from the Stick view and seen mounted again is no longer "safe to remove" (wbx-72), whichever view shows.
  useEffect(() => { stickDevicesSeen(devices); }, [devices]);
  // What each stick's own libraries hold, read when a stick is opened.
  // Off in the view shell: Stick edits never read back through the recovering path (wbx-45.4).
  const stickReadBack = useRef(true);
  const deviceLibraries = useDeviceLibraries(devices, stickReadBack);
  const [syncing, setSyncing] = useState(false);
  const [ejectingDeviceId, setEjectingDeviceId] = useState<string | null>(null);
  const ejectingDeviceRef = useRef(false);
  // Closed by default, which is what browseSetting.xml records for the user's
  // own rekordbox (`ListInfo open="0"`).
  const [infoOpen, setInfoOpen] = useState(restored.infoOpen);
  // The My Tag window takes the information window's place: opening one
  // closes the other, as rekordbox's icon column does [OBS 7.2.14, #539].
  const [myTagOpen, setMyTagOpen] = useState(restored.myTagOpen);
  // Closed unless it was open at exit; browseSetting.xml records
  // `SubBrowse open="0"` for a first run.
  const [subOpen, setSubOpen] = useState(restored.subOpen);
  // The sub-browser's width and its tree's, as last dragged. Clamped to the
  // window by the panel itself, so these are what was asked for.
  const [subWidth, setSubWidth] = useState(restored.subWidth);
  const [subTreeWidth, setSubTreeWidth] = useState(restored.subTreeWidth);
  // The Track Filter bar. Its open state is kept across runs, its picks are
  // not: a library that comes back already narrowed reads as a broken one.
  const [filterOpen, setFilterOpen] = useState(restored.filterOpen);
  const [filterState, setFilterState] = useState<FilterState>(EMPTY_FILTER);
  const [filterValues, setFilterValues] = useState<FilterValues | null>(null);
  // Why the library is not there, when it is not. Shown instead of "Loading…",
  // which is a lie once the load has failed.
  const [loadError, setLoadError] = useState<string | null>(null);
  // What to ask when there is no library to load: none anywhere, or one
  // configured on a drive that is not connected.
  const [missingLibrary, setMissingLibrary] = useState<LibraryQuestion | null>(null);
  /** File › Display All Missing Files: the Missing File Manager is open. */
  const [missingFilesOpen, setMissingFilesOpen] = useState(false);
  const [summary, setSummary] = useState<LibrarySummary | null>(null);
  // Do not write the empty bootstrap selection over the session while the
  // backend is still restoring the node that was open at exit. WebKit gets
  // to the effect before the mock backend answers, which exposed the same
  // race a slower real library can hit.
  const [sessionReady, setSessionReady] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  // Seed the selected node along with the cached tree. Besides avoiding an
  // unnecessary blank title, this keeps the restored view coherent until the
  // backend replaces both with current data.
  const [selectedNode, setSelectedNode] = useState<TreeNode | null>(() =>
    restored.tree.find((node) => node.id === restored.selectedNodeId) ?? null,
  );
  // One piece of state, not two: updating `descending` from inside a `setSort`
  // updater made the toggle a side effect, and StrictMode's double invocation
  // cancelled it out.
  // Opens in the view's own order: a playlist's is the order somebody put it
  // in, and the collection has no more meaningful default.
  const [sortState, setSortState] = useState<SortState>(restored.sort);
  const [selectedCount, setSelectedCount] = useState(0);
  const [listTotals, setListTotals] = useState<ListTotals | undefined>(undefined);
  // Whether the tracks were picked in the list now open; see `TrackTable`.
  const [barShowsSelection, setBarShowsSelection] = useState(false);
  const changeSelectedCount = useCallback((count: number, picked: boolean) => {
    setSelectedCount(count);
    setBarShowsSelection(picked);
  }, []);
  // The rows behind the selection, so they can be queued for analysis.
  const [selectedTracks, setSelectedTracks] = useState<{ id: string; title: string }[]>([]);
  // The row the player is showing. Set by the browser as the selection moves,
  // so the player reflects what is highlighted rather than nothing. It starts
  // empty on every run: a track restored from the last session would be loaded
  // before the library is read, which fails, and the deck would open on an
  // error over something nobody asked to hear.
  const [playerTrack, setPlayerTrack] = useState<RowDto | null>(null);
  // Deck B, which only exists in the two-player layouts. Loaded by dropping a
  // track on it: the browser's selection drives deck A alone, or picking
  // through a playlist would keep replacing whatever B was cued to.
  const [playerTrackB, setPlayerTrackB] = useState<RowDto | null>(null);
  // The one row the browser has selected, so an empty deck can be clicked to
  // take it. Selecting still loads nothing by itself.
  const [selectedRow, setSelectedRow] = useState<RowDto | null>(null);
  // The track ids the information panel follows: the selection of whichever
  // browser list changed last, in the order that list reports it. More than
  // one is a multiple selection, which the panel edits as a whole.
  const [infoSelection, setInfoSelection] = useState<readonly string[]>([]);
  // Where each deck's transport is drawn in the two-deck layouts. State rather
  // than a ref, because the players have to re-render once the slots exist.
  const [transportA, setTransportA] = useState<HTMLDivElement | null>(null);
  const [transportB, setTransportB] = useState<HTMLDivElement | null>(null);
  /**
   * DUAL CONTROL: one zoom and one beat-jump size for both decks.
   *
   * Off, each deck keeps its own; on, the shell holds them and hands the same
   * value to both, so a wheel over one waveform moves the other with it.
   */
  /**
   * Which deck the other syncs to, and each deck's BEAT SYNC.
   *
   * No deck is the master until one plays; the first to play takes it, as
   * rekordbox's EXPORT mode does. BEAT SYNC is each deck's own toggle, the
   * master's included; a lit deck follows only while the other deck is the
   * master. See `@/lib/syncMaster` for the evidence.
   */
  const [sync, setSync] = useState<SyncState>(INITIAL_SYNC);
  const syncMaster = sync.master;
  const setSyncMaster = useCallback((deck: DeckId) => setSync((s) => pressMaster(s, deck)), []);
  const toggleSync = useMemo(
    () => ({
      a: () => setSync((s) => toggleBeatSync(s, "a")),
      b: () => setSync((s) => toggleBeatSync(s, "b")),
    }),
    [],
  );
  const startedPlaying = useMemo(
    () => ({
      a: () => setSync((s) => deckStartedPlaying(s, "a")),
      b: () => setSync((s) => deckStartedPlaying(s, "b")),
    }),
    [],
  );
  /**
   * What each deck is playing at — its file's BPM times its tempo — so the
   * synced deck can be handed the master's and re-match when it moves.
   */
  const [playingBpm, setPlayingBpm] = useState<Record<DeckId, number | null>>({ a: null, b: null });
  const reportPlayingBpm = useMemo(
    () => ({
      a: (bpm: number | null) => setPlayingBpm((p) => (p.a === bpm ? p : { ...p, a: bpm })),
      b: (bpm: number | null) => setPlayingBpm((p) => (p.b === bpm ? p : { ...p, b: bpm })),
    }),
    [],
  );
  const leaderBpmX100 = syncMaster === null ? null : playingBpm[syncMaster];
  /** Each deck's key shift in semitones, so the Traffic Light reads the key the deck sounds in. */
  const [keyShift, setKeyShift] = useState<Record<DeckId, number>>({ a: 0, b: 0 });
  const reportKeyShift = useMemo(
    () => ({
      a: (semitones: number) => setKeyShift((k) => (k.a === semitones ? k : { ...k, a: semitones })),
      b: (semitones: number) => setKeyShift((k) => (k.b === semitones ? k : { ...k, b: semitones })),
    }),
    [],
  );
  /**
   * How each deck reads the other for sync.
   *
   * Getters rather than state: a deck's position moves every frame and sync
   * reads it once, when the button goes down. Holding it as state would
   * re-render the shell sixty times a second for a number nobody is looking
   * at.
   */
  const syncA = useRef<() => SyncDeck | null>(() => null);
  const syncB = useRef<() => SyncDeck | null>(() => null);
  const publishSync = useMemo(
    () => ({
      a: (get: () => SyncDeck | null) => {
        syncA.current = get;
      },
      b: (get: () => SyncDeck | null) => {
        syncB.current = get;
      },
    }),
    [],
  );
  const peerSync = useMemo(
    () => ({ a: () => syncB.current(), b: () => syncA.current() }),
    [],
  );
  /**
   * A grid shift on one deck, handed to the other: a deck synced to the
   * shifted one moves with it. Each deck decides by its own BEAT SYNC
   * whether it moves.
   */
  const followA = useRef<(ms: number) => void>(() => {});
  const followB = useRef<(ms: number) => void>(() => {});
  const publishGridFollow = useMemo(
    () => ({
      a: (follow: (ms: number) => void) => {
        followA.current = follow;
      },
      b: (follow: (ms: number) => void) => {
        followB.current = follow;
      },
    }),
    [],
  );
  const gridNudged = useMemo(
    () => ({ a: (ms: number) => followB.current(ms), b: (ms: number) => followA.current(ms) }),
    [],
  );
  /**
   * The zoom cluster the two-deck layout shares, registered the same way:
   * one + RST − over the line where the two details meet, and a press
   * zooms both decks. The wheel over either detail is the same press, as
   * rekordbox's is (#602). DUAL CONTROL off, each deck still holds its own
   * zoom, and a step moves each from where it is; RST brings both back to
   * the default.
   */
  const zoomA = useRef<(by: number) => void>(() => {});
  const zoomB = useRef<(by: number) => void>(() => {});
  const publishZoom = useMemo(
    () => ({
      a: (zoom: (by: number) => void) => {
        zoomA.current = zoom;
      },
      b: (zoom: (by: number) => void) => {
        zoomB.current = zoom;
      },
    }),
    [],
  );
  const zoomBoth = useCallback((by: number) => {
    zoomA.current(by);
    zoomB.current(by);
  }, []);
  // Remembered across runs: a DUAL CONTROL left on comes back on.
  const [dual, setDual] = useState(restored.dualControl);
  const [waveformZoom, setWaveformZoom] = useState(restored.waveformZoom);
  const setZoomA = useCallback((bars: number) => {
    setWaveformZoom((zoom) => zoom.a === bars ? zoom : { ...zoom, a: bars });
  }, []);
  const setZoomB = useCallback((bars: number) => {
    setWaveformZoom((zoom) => zoom.b === bars ? zoom : { ...zoom, b: bars });
  }, []);
  const [dualBars, setDualBars] = useState(restored.waveformZoom.a);
  const setLinkedZoom = useCallback((bars: number) => {
    setDualBars(bars);
    setWaveformZoom({ a: bars, b: bars });
  }, []);
  // RST: both decks back to the default zoom, linked or not (#602).
  const resetZoom = useCallback(() => setLinkedZoom(DETAIL_BARS), [setLinkedZoom]);
  // Remembered across runs the same way: each deck's beat-jump size. Linked,
  // both decks read deck A's and a pick is written to both, so what is shown
  // is always what is stored and comes back the same after a restart.
  const [jumpSize, setJumpSize] = useState(restored.jumpSize);
  const setJumpA = useCallback((id: string) => {
    setJumpSize((size) => size.a === id ? size : { ...size, a: id });
  }, []);
  const setJumpB = useCallback((id: string) => {
    setJumpSize((size) => size.b === id ? size : { ...size, b: id });
  }, []);
  const setLinkedJump = useCallback((id: string) => {
    setJumpSize({ a: id, b: id });
  }, []);
  const [query, setQuery] = useState("");
  const [searchField, setSearchField] = useState<TrackSearchField>("all");
  // The tree's width, dragged by the splitter. Held here because the grid that
  // sizes both panes lives here.
  const [treeWidth, setTreeWidth] = useState(restored.treeWidth);
  const bodyRef = useRef<HTMLDivElement>(null);
  const dragFrom = useRef<{ x: number; width: number } | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const clock = useClock();
  // The table's layout follows the kind of thing being browsed, as
  // browseSetting.xml does, rather than each individual playlist.
  const columnContext: ColumnContext = columnContextFor(selectedNode?.kind);
  /**
   * The artwork display pattern, the two buttons beside MASTER. The main
   * browser and the sub-browser each have their own pair and their own mode
   * (rekordbox's `ListViewer` holds an `ArtworkStatus` each), and both keep
   * the last one chosen under one name, `ArtworkStatus-TrackList`, which both
   * start from [OBS static, rekordbox 7.2.19: `SubTrackListHeader` makes both
   * buttons and its `getHeaderType` is 0, the main list's].
   */
  const [artworkMode, chooseArtworkMode] = useState<ArtworkMode>(restored.artworkMode);
  const [subArtworkMode, chooseSubArtworkMode] = useState<ArtworkMode>(restored.artworkMode);
  const [savedArtworkMode, setSavedArtworkMode] = useState<ArtworkMode>(restored.artworkMode);
  const setArtworkMode = useCallback((mode: ArtworkMode) => {
    chooseArtworkMode(mode);
    setSavedArtworkMode(mode);
  }, []);
  const setSubArtworkMode = useCallback((mode: ArtworkMode) => {
    chooseSubArtworkMode(mode);
    setSavedArtworkMode(mode);
  }, []);
  // Each mode has its own columns, as rekordbox keeps a `-FullArtwork`
  // layout beside each table's.
  const cols = useColumns(columnContext, artworkMode);
  // The view shell's Library has its own columns and row look (#55).
  const libraryColumns = useLibraryColumns();
  // The Preferences window, and the pane it opens on: the missing-file
  // manager lives under Advanced, so the File menu opens it there. In the
  // shell it is a window of its own; in a browser, which has no windows to
  // open, it is drawn over this one.
  const [settingsOpen, setSettingsOpen] = useState<PreferencesTarget | null>(null);
  const openPreferences = useCallback((pane: PreferencesTarget) => {
    void getBackend().then(async (backend) => {
      const opened = await backend.openPreferences(pane).catch(() => false);
      if (!opened) setSettingsOpen(pane);
    });
  }, []);
  // The Sync Manager, the same way: a window of its own in the shell, and
  // drawn over this one in a browser.
  const [syncOpen, setSyncOpen] = useState(false);
  const openSyncManager = useCallback(() => {
    void getBackend().then(async (backend) => {
      const opened = await backend.openSyncWindow().catch(() => false);
      if (!opened) setSyncOpen(true);
    });
  }, []);
  // The Log window, the same way again: clicking the status bar's message
  // opens it, or brings the open one to the front.
  const [logOpen, setLogOpen] = useState(false);
  const openLogWindow = useCallback(() => {
    void getBackend().then(backend => backend.openLogWindow()).then(opened => { if (!opened) setLogOpen(true); })
      .catch(() => setLogOpen(true));
  }, []);
  const prefs = usePreferencesStore();
  const { view: viewPrefs, advanced: advancedPrefs, analysis: analysisPrefs } = prefs.preferences;
  const { palette: waveformPalette } = useEffectiveWaveform(viewPrefs.waveformColor);
  // Every write path reads this one flag: rekordbox holding the database,
  // or Library Protection in Preferences, refuse the same way.
  const backupJob = useBackupProgress();
  const exportJobs = useExportProgress();
  const exportBatch = [...exportJobs.values()];
  const exportRunning = exportBatch.some(isExportActive);
  const readOnly = (summary?.readOnly ?? false) || advancedPrefs.protectLibrary;
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try {
        const backend = await getBackend();
        if (!live) return;
        const info = await backend.librarySummary();
        if (live) {
          setSummary((current) => current && current.readOnly !== info.readOnly
            ? { ...current, readOnly: info.readOnly }
            : current);
        }
      } catch {
        // Keep the last known lock state if the library is temporarily unavailable.
      } finally {
        if (live) timer = setTimeout(() => void refresh(), 2000);
      }
    };
    timer = setTimeout(() => void refresh(), 2000);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let live = true;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stop = backend.onEditHistory((history) => {
        setEditHistory(history);
        setLibraryEditHistory(history.undoLabel, history.redoLabel);
      });
    })();
    return () => {
      live = false;
      stop?.();
      setLibraryEditHistory(null, null);
    };
  }, []);
  // DJ System in Preferences is what a stick with no settings of its own
  // gets on export; the same shape goes with every export call.
  const stickDefaults = prefs.preferences.djSystem;
  const deleteUnlistedMusic = prefs.preferences.usbExport.deleteUnlistedMusic;
  const compatibilityFormat = prefs.preferences.usbExport.maximumCompatibility ? prefs.preferences.usbExport.conversionFormat : undefined;
  /** How much of the window the deck takes, kept across restarts. */
  const [chosenLayout, setLayout] = useState<PlayerLayout>(restored.layout);
  /**
   * The view shell (ticket #50): the four-view frame drawn in place of the
   * layout below, behind a switch that is off unless turned on. Its player
   * area holds one deck at its smallest until ticket #51 builds the compact
   * player, so while the shell is on the deck layout is that one, whatever
   * the layout menu last chose; the choice itself is kept for the old layout.
   */
  const shellOn = prefs.preferences.view.viewShell;
  // Set during render, before any effect of this commit can read it.
  stickReadBack.current = !shellOn;
  const intakeCounts = useIntakeCounts(shellOn);
  const reviewCount = useReviewCount(shellOn);
  // The views loaded on demand (wbx-45.13): fetched once the shell has painted.
  useEffect(() => { if (shellOn) { const at = setTimeout(preloadShellViews, 0); return () => clearTimeout(at); } return undefined; }, [shellOn]);
  // The shell's tree reads each Stick's playlists without writing to it
  // (ticket #52); only drawn, so only read, with the shell on.
  // Read in both layouts: the Stick write gate checks every Stick write against them (wbx-45.4).
  const shellSticks = useStickOutlines(devices, deviceLibraries.revision);
  // Stick Protection (wbx-45.11), shown in the tree, the status line and the Stick's panel.
  const stickProtection = useStickProtection(devices);
  const [stickProtectionOpen, setStickProtectionOpen] = useState(false);
  const shellSticksShown = useMemo(() => shellSticks.map((stick) => {
    const found = stickProtection.get(stick.path);
    return found === undefined ? stick : { ...stick, protection: found.protected ? "protected" as const : "unlocked" as const };
  }), [shellSticks, stickProtection]);
  const layout: PlayerLayout = shellOn ? SHELL_PLAYER_LAYOUT : chosenLayout;
  /** The view in the shell's working area. Here, so it outlives nothing it should not. */
  const [shellView, setShellView] = useState<ShellView>(DEFAULT_SHELL_VIEW);
  /**
   * The Traffic Light: which deck's key the browser lights rows against —
   * the MASTER menu above the track list. The key itself is that deck's
   * loaded track's, so it follows the master when the master moves.
   */
  const [trafficLight, setTrafficLight] = useState<TrafficLightSource>(restored.trafficLight);
  const activeTrafficLight = trafficLight === "b" && deckCount(layout) < 2 ? "a" : trafficLight;
  const trafficDeck: DeckId = deckCount(layout) < 2 ? "a" : activeTrafficLight === "master" ? (syncMaster ?? "a") : activeTrafficLight;
  /*
   * A new track on the master deck, or none, hands MASTER to the other deck
   * when that one holds a track, as rekordbox does: the deck that was
   * following keeps the tempo it was playing at, rather than jumping to the
   * new track's BPM [OBS rekordbox 7 Export, chris-win11, parity/issue-128;
   * manual p.168 "When changing or unloading a track on the deck of the sync
   * master the sync master is switched to the other deck"]. Its BEAT SYNC is
   * left as it was, so it follows again if MASTER comes back.
   */
  const masterTrackId = (syncMaster === "b" ? playerTrackB : playerTrack)?.id ?? null;
  const otherTrackId = (syncMaster === "b" ? playerTrack : playerTrackB)?.id ?? null;
  const twoDecks = deckCount(layout) >= 2;
  const masterHeld = useRef({ deck: syncMaster, track: masterTrackId });
  useEffect(() => {
    const held = masterHeld.current;
    masterHeld.current = { deck: syncMaster, track: masterTrackId };
    if (syncMaster === null || held.deck !== syncMaster || held.track === null || held.track === masterTrackId) return;
    if (!twoDecks || otherTrackId === null) return;
    setSyncMaster(syncMaster === "a" ? "b" : "a");
  }, [syncMaster, masterTrackId, otherTrackId, twoDecks, setSyncMaster]);
  const trafficTrack = trafficDeck === "b" ? playerTrackB : playerTrack;
  const trafficKey = trafficTrack ? transposeKey(trafficTrack.key, keyShift[trafficDeck]) : null;
  const master = useMasterControls();
  // Read at start so the remembered setting reaches the engine before the
  // first thing plays, not when Settings is next opened.
  const limiter = useLimiter();

  // Preferences › Audio, handed to the engine: the rate and buffer the
  // device is opened with, and the metronome's click. On start and on every
  // change, from this window or the Preferences window's storage event.
  const audioPrefs = prefs.preferences.audio;
  useAudioOutput(audioPrefs.outputDevice);
  useEffect(() => {
    void (async () => {
      const backend = await getBackend();
      try {
        await backend.setAudioConfig(audioPrefs.sampleRate, audioPrefs.bufferSize);
        await backend.setMetronome(audioPrefs.metronomeSound, audioPrefs.metronomeVolume);
      } catch (e) {
        console.warn("the audio settings did not reach the engine", e);
      }
    })();
  }, [audioPrefs]);
  // The transport edge on its own: live, so a change must not also re-send
  // the device settings. Both layouts and both players share the engine's.
  const transportEdge = audioPrefs.transportEdge;
  useEffect(() => {
    void (async () => {
      try {
        const backend = await getBackend();
        await backend.setTransportEdge(transportEdge);
      } catch (e) {
        console.warn("the transport edge did not reach the engine", e);
      }
    })();
  }, [transportEdge]);
  useEffect(() => {
    let live = true;
    void getBackend()
      .then((backend) => backend.appVersion())
      .then((found) => {
        if (live) setVersion(found);
      })
      .catch(() => {
        // A build with no shell behind it has no version; the strip just
        // shows the name.
      });
    return () => {
      live = false;
    };
  }, []);
  // Long library operations wait their turn here, shown in the status bar.
  const jobError = useRef<(job: JobView, error: unknown) => void>(() => undefined);
  const [jobs] = useState(() => new JobQueue((job, error) => jobError.current(job, error)));
  const jobList = useSyncExternalStore(jobs.subscribe, jobs.getSnapshot);
  // An analysed track's BPM, key and waveform change: its row is drawn from
  // the answer at once, and the library is re-read once the run is over so
  // every view holds what was written. A running job that moves files (Auto
  // Relocate) holds the run: the status bar says it is queued, and no track
  // is read from where its file used to be (#286).
  const analysisHeld = jobList.some((job) => job.holdsAnalysis && job.state !== "queued");
  const analysis = useAnalysis(
    useCallback((id: string, result: AnalysisResult) => {
      const found = {
        analysed: result.analysed ?? 1,
        bpmX100: result.bpmX100,
        key: result.key,
        durationSec: result.durationSec,
      };
      setPendingEdits((edits) => new Map(edits).set(id, found));
      // A deck holds the row it was loaded with: one holding this track
      // takes the answer too, so it keeps the waveform it was drawing and
      // gains the tempo and key.
      const update = (row: RowDto | null) => (row?.id === id ? { ...row, ...found } : row);
      setPlayerTrack(update);
      setPlayerTrackB(update);
    }, []),
    useCallback(() => {
      void getBackend().then((backend) => backend.reloadLibrary());
    }, []),
    analysisPrefs,
    analysisHeld,
  );
  // What is in flight out of the browser: a playlist takes the ids, a deck
  // takes the one row under the hand.
  const [draggedTracks, setDraggedTracks] = useState<TrackDrag | null>(null);
  /**
   * The last thing the app has to say, and whether it went wrong.
   *
   * One channel, two colours: "Added 3 tracks" and "rekordbox is running, so
   * the library is open read-only" arrive the same way and are not the same
   * kind of news.
   */
  const [note, setNote] = useState<{ text: string; failed: boolean; busy?: boolean } | null>(null);
  const [playerError, setPlayerError] = useState<string | null>(null);
  const report = useCallback((text: string) => setNote({ text, failed: false }), []);
  const refuse = useCallback((text: string) => setNote({ text, failed: true }), []);
  useEffect(() => {
    if (analysis.recordingError) refuse(analysis.recordingError);
  }, [analysis.recordingError, refuse]);
  const stopJob = useCallback((id: number) => jobs.stop(id), [jobs]);
  // The Library service this window hosts has stopped, or another window
  // has the Library: nothing more is saved, and the status bar says why for
  // as long as that is so. Asked as well as heard, since it can happen
  // before this has subscribed.
  const [serviceProblem, setServiceProblem] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stop = backend.onServiceStopped(setServiceProblem);
      const problem = await backend.serviceProblem().catch(() => null);
      if (live && problem) setServiceProblem(problem);
    })();
    return () => {
      live = false;
      stop?.();
    };
  }, []);
  // Library Protection belongs to the backend, which refuses every change
  // while it is on; the preference shows what the backend last said.
  const updateAdvanced = prefs.update;
  useLibraryProtection(
    advancedPrefs.protectLibrary,
    useCallback((on: boolean) => updateAdvanced("advanced", { protectLibrary: on }), [updateAdvanced]),
  );
  jobError.current = (job, error) => refuse(error instanceof Error ? error.message : `${job.label} failed.`);
  // A waveform click whose track could not be previewed says why.
  useEffect(() => onPreviewError(refuse), [refuse]);
  // Choosing another playlist in the tree stops the preview, as rekordbox's
  // `BrowseListViewer::currentBrowseChanged` does when the tree's selected
  // item changed. The row with its stop button is gone with the old list,
  // and a preview left playing had nothing left to stop it (#242).
  const previewedNode = useRef(selectedNode?.id ?? null);
  useEffect(() => {
    const id = selectedNode?.id ?? null;
    if (id === previewedNode.current) return;
    previewedNode.current = id;
    void stopPreview();
  }, [selectedNode?.id]);
  const openLog = useCallback(() => {
    void getBackend()
      .then((backend) => backend.openLog())
      .catch((error) => refuse(error instanceof Error ? error.message : String(error)));
  }, [refuse]);
  useEffect(() => {
    setNote((current) => {
      if (!current?.failed) return current;
      if (summary?.readOnly === false && current.text === refusal(false)) return null;
      if (!advancedPrefs.protectLibrary && current.text === refusal(true)) return null;
      return current;
    });
  }, [summary?.readOnly, advancedPrefs.protectLibrary]);
  const explainEditLock = useCallback(() => {
    refuse(refusal(advancedPrefs.protectLibrary));
  }, [refuse, advancedPrefs.protectLibrary]);
  // Bumped whenever the library changes underneath us, which drops cached
  // pages. Without it an edit's effect never reached the table.
  const [libraryGeneration, setLibraryGeneration] = useState(0);
  // Tag List edits keep the generation, so the filter bar over the Tag List
  // follows this instead.
  const [tagListRevision, setTagListRevision] = useState(0);
  // Edits shown at once, dropped when the backend's reload lands. A write
  // makes the backend re-read the library — 243 ms on the real collection —
  // and waiting for that before a star fills in feels broken.
  const [pendingEdits, setPendingEdits] = useState<Map<string, Partial<RowDto>>>(
    () => new Map(),
  );
  // Read once: the platform cannot change while the window is open, and
  // deciding it per key press would run a regex on every stroke.
  const platform = useMemo(detectPlatform, []);

  useEffect(() => {
    let cancelled = false;
    let stopReady: (() => void) | undefined;
    let stopProblem: (() => void) | undefined;

    /** One attempt at the first load. False means the library is not up yet. */
    const attempt = async (backend: Backend) => {
      try {
        // Devices are deliberately not in here. A stick that cannot be read
        // must not stop the collection from appearing, and it used to: this
        // was one `Promise.all`, so any of the three failing left the window
        // on "Loading…" with nothing said.
        const [nodes, info] = await Promise.all([
          backend.playlistTree(),
          backend.librarySummary(),
        ]);
        if (cancelled) return true;
        setTree(withSources(nodes));
        setSummary(info);
        setLoadError(null);
        setMissingLibrary(null);
        // The playlist that was open at exit, when it is still there — it can
        // have been deleted between runs, so this is a lookup, not a promise.
        const remembered = nodes.find((n) => n.id === restored.selectedNodeId);
        setSelectedNode(remembered ?? nodes.find((n) => n.kind === "playlist") ?? nodes[0] ?? null);
        setSessionReady(true);
        void backend
          .listDevices()
          .then((volumes) => {
            if (!cancelled) setDevices(volumes);
          })
          .catch(() => {
            // Nothing to say: no devices is the normal case.
          });
        return true;
      } catch {
        // The usual reason is that the backend is still reading the library,
        // which the ready event below will tell us about.
        return false;
      }
    };

    void (async () => {
      const backend = await getBackend();
      if (cancelled) return;
      // Subscribed before the first attempt, not after. The library can become
      // ready in the gap between a failed attempt and a later subscription,
      // and that gap is exactly where the window used to get stuck.
      stopReady = backend.onLibraryReady(() => {
        void attempt(backend);
      });
      const applyProblem = (problem: LibraryProblem | null) => {
        if (cancelled || problem === null) return;
        if (problem.kind === "failed") {
          // A library that is there and would not open is reported, not asked about.
          setMissingLibrary(null);
          setLoadError(problem.message);
        } else {
          setMissingLibrary(problem);
        }
      };
      stopProblem = backend.onLibraryProblem(applyProblem);
      // Asked as well: with no library at all the backend gives up before
      // this window has subscribed, and the event is gone.
      if (!(await attempt(backend))) applyProblem(await backend.libraryProblem().catch(() => null));
    })();

    return () => {
      cancelled = true;
      stopReady?.();
      stopProblem?.();
    };
    // `restored` is read once and never changes, but the rule cannot know that
    // and the id is genuinely read here.
  }, [restored.selectedNodeId]);

  // LINK, for the status bar: on or off and how many players are on it.
  // Read once and then by event, so the strip follows Preferences' switch
  // without owning it.
  const [link, setLink] = useState<LinkStatus | null>(null);
  const [linkPeers, setLinkPeers] = useState<LinkPeerSeen[]>([]);
  const [linkBusy, setLinkBusy] = useState(false);
  useEffect(() => {
    let live = true;
    const stops: Array<() => void> = [];
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stops.push(backend.onLinkStatus((status) => live && setLink(status)));
      stops.push(backend.onLinkPeers((peers) => live && setLinkPeers(peers)));
      const [status, peers] = await Promise.all([backend.linkStatus(), backend.linkPeers()]);
      if (live) {
        setLink(status);
        setLinkPeers(peers);
      }
      // Re-check why LINK is off when the window regains focus: quitting
      // rekordbox frees the ports, and the "unavailable" warning should clear
      // without a restart rather than lingering after the reason is gone.
      const refresh = () => {
        void backend.linkStatus().then((s) => live && setLink(s));
      };
      window.addEventListener("focus", refresh);
      stops.push(() => window.removeEventListener("focus", refresh));
    })();
    return () => {
      live = false;
      stops.forEach((stop) => stop());
    };
  }, []);

  // On the interface chosen under DJ System, or the one the players are
  // reached through when none is.
  const linkInterface = stickDefaults.linkInterface;
  /** Turns LINK on or off and shows what came of it; the status says why not. */
  const setLinkOn = useCallback(async (on: boolean): Promise<LinkStatus> => {
    setLinkBusy(true);
    const backend = await getBackend();
    try {
      const status = on ? await backend.startLinkExport(
        linkInterface ?? undefined,
        {
          waveformColor: stickDefaults.waveformColor,
          waveformPosition: stickDefaults.waveformPosition,
          overviewWaveform: stickDefaults.overviewWaveform,
          keyDisplay: stickDefaults.keyDisplay,
        },
        stickDefaults.linkKeySort,
      ) : await backend.stopLinkExport();
      setLink(status);
      return status;
    } finally {
      setLinkBusy(false);
    }
  }, [linkInterface, stickDefaults.keyDisplay, stickDefaults.linkKeySort,
    stickDefaults.overviewWaveform, stickDefaults.waveformColor, stickDefaults.waveformPosition]);
  const toggleLink = useCallback(() => {
    void setLinkOn(!link?.on);
  }, [link?.on, setLinkOn]);
  const autoStartLink = useCallback(() => setLinkOn(true), [setLinkOn]);

  useAutoJoinLink(
    stickDefaults.autoJoinLink,
    linkPeers,
    link,
    linkBusy,
    autoStartLink,
  );

  // The tempo-master controls: each returns LINK's fresh status.
  const setLinkMaster = useCallback((on: boolean) => {
    void (async () => setLink(await (await getBackend()).setLinkMaster(on)))();
  }, []);
  const nudgeLinkMaster = useCallback((deltaBpm: number) => {
    void (async () => setLink(await (await getBackend()).nudgeLinkMaster(deltaBpm)))();
  }, []);
  const takeLinkMasterTempo = useCallback(() => {
    void (async () => setLink(await (await getBackend()).takeLinkMasterTempo()))();
  }, []);

  // The master player's BPM for the filter's `MASTER PLAYER ±` list: the
  // track on whichever deck is MASTER. `[ASSUME]` the track's own BPM, not
  // the deck's tempo-adjusted one — the tempo lives in the player and the
  // capture cannot say which rekordbox uses.
  // What each deck holds, Deck 1 first, for Track › Locate track loaded on
  // deck; and All Tracks, where it looks for a track the open list lacks
  // (rekordbox selects Collection).
  const deckTrackIds = useMemo(() => [playerTrack?.id ?? null, playerTrackB?.id ?? null], [playerTrack?.id, playerTrackB?.id]);
  // With All Tracks hidden from the tree (View › Layout), the Playlists
  // heading lists the same collection.
  const showCollection = useCallback(() => {
    const target = (viewPrefs.allTracks ? tree.find((item) => item.kind === "allTracks") : undefined)
      ?? tree.find((item) => item.kind === "collection");
    if (target) setSelectedNode(target);
  }, [tree, viewPrefs.allTracks]);
  const masterBpmX100 = (syncMaster === "b" ? playerTrackB : playerTrack)?.bpmX100 ?? null;
  // Related Tracks relate to the track on Player 1, as rekordbox's do.
  const relatedTo = playerTrack?.id ?? null;
  // The shell's Stick writes are staged and confirmed here (wbx-45.4). Its
  // refusal also keeps the list from opening a Stick playlist (a read that
  // settles an unfinished write) while the Stick is not known to be settled.
  const stickGate = useStickWriteGate({ sticks: shellSticks, refresh: deviceLibraries.refresh, refuse });
  const listNode = shellOpensNode(selectedNode, stickGate.refusal);
  // The view shell's Track Filter bar (wbx-57): always shown above the list,
  // its text is the list's search. Of the node the list may open, as the list.
  const shellFilterBase: ViewSpec = useMemo(() => ({ ...specForNode(listNode, "", null, "classic", "alphabetical", relatedTo, deviceLibraries.revision), searchField }),
    [listNode, relatedTo, deviceLibraries.revision, searchField]);
  // Key match and BPM ±6% follow the track the key lights follow (wbx-57).
  const shellFilterLoaded = useMemo(() => ({ key: trafficKey || null, bpmX100: trafficTrack?.bpmX100 ?? null }), [trafficKey, trafficTrack?.bpmX100]);
  const shellFilter = useShellTrackFilter({ enabled: shellOn, base: shellFilterBase, query, setQuery, generation: libraryGeneration, tagListRevision, loaded: shellFilterLoaded });
  // Selected tracks the filter hid and the list deselected, as last told;
  // said until the filter is cleared.
  const [filterDeselected, setFilterDeselected] = useState(0);
  const shellFilterChange = shellFilter.onChange;
  const onShellFilterChange = useCallback((next: Parameters<typeof shellFilterChange>[0]) => {
    if (!hasPicks(next) && next.text.trim() === "") setFilterDeselected(0);
    shellFilterChange(next);
  }, [shellFilterChange]);
  const spec: ViewSpec = useMemo(() => {
    const base = { ...specForNode(listNode, query, sortState, viewPrefs.keyDisplay, viewPrefs.keySort, relatedTo, deviceLibraries.revision), searchField };
    // Only while the bar is showing: hiding it puts the whole list back,
    // so a closed bar can never be silently narrowing the library.
    const filter = shellOn ? shellFilter.viewFilter : filterOpen ? toSpecFilter(filterState, masterBpmX100) : undefined;
    return filter ? { ...base, filter } : base;
  }, [listNode, sortState, query, searchField, filterOpen, filterState, masterBpmX100, viewPrefs.keyDisplay, viewPrefs.keySort, relatedTo, deviceLibraries.revision, shellOn, shellFilter.viewFilter]);

  // What the bar's lists offer, from Rust, for the source and query alone.
  // Re-asked when either changes or the library does, and only while the bar
  // is open — a closed bar costs nothing.
  const tagListKey = selectedNode?.kind === "tagList" ? tagListRevision : 0;
  useEffect(() => {
    if (!filterOpen) return;
    let live = true;
    void (async () => {
      const backend = await getBackend();
      try {
        const values = await backend.filterValues({ ...specForNode(selectedNode, query, null, "classic", "alphabetical", relatedTo), searchField });
        if (live) setFilterValues(values);
      } catch {
        // The library is not up yet; the ready event re-runs this through
        // `libraryGeneration`.
      }
    })();
    return () => {
      live = false;
    };
  }, [filterOpen, selectedNode, query, searchField, libraryGeneration, relatedTo, tagListKey]);

  const handleSort = useCallback((column: SortColumn) => {
    setSortState((s) => nextSort(s, column));
  }, []);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let stopTagList: (() => void) | undefined;
    let live = true;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stopTagList = backend.onTagListChanged(() => setTagListRevision((n) => n + 1));
      stop = backend.onLibraryChanged((generation) => {
        setLibraryGeneration(generation);
        // The reload carries the edits, so the overlay has done its job.
        setPendingEdits(new Map());
        // The tree can change too — a playlist gained tracks, or one was
        // deleted — so it is re-read rather than assumed still right.
        void backend.playlistTree().then((nodes) => setTree(withSources(nodes)));
      });
    })();
    return () => {
      live = false;
      stop?.();
      stopTagList?.();
    };
  }, []);

  const bounds = useCallback(
    () => ({ ...TREE_BOUNDS, available: bodyRef.current?.clientWidth ?? 0 }),
    [],
  );

  // Re-clamp when the window changes: a width that fitted a wide window can
  // leave the track list with nothing in a narrow one.
  useEffect(() => {
    const body = bodyRef.current;
    if (!body || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      setTreeWidth((w) => clampWidth(w, bounds()));
    });
    observer.observe(body);
    return () => {
      observer.disconnect();
    };
  }, [bounds]);

  const onSplitterDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      dragFrom.current = { x: event.clientX, width: treeWidth };
      // Capture, so the drag survives the pointer leaving the 4px handle.
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    [treeWidth],
  );

  const onSplitterMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const from = dragFrom.current;
      if (!from) return;
      setTreeWidth(clampWidth(from.width + (event.clientX - from.x), bounds()));
    },
    [bounds],
  );

  const onSplitterUp = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    dragFrom.current = null;
    event.currentTarget.releasePointerCapture(event.pointerId);
  }, []);

  /** Runs one edit and reports what happened, refusals included; resolves whether it was written. */
  const writeEdit = useCallback(
    async (what: string, edit: (b: Awaited<ReturnType<typeof getBackend>>) => Promise<unknown>): Promise<boolean> => {
      // Library Protection is a choice the backend cannot see, so it is
      // refused here, with the same words the menu uses. rekordbox holding
      // the database is the backend's to refuse, checked as the write starts.
      if (advancedPrefs.protectLibrary) {
        refuse(refusal(true));
        return false;
      }
      const backend = await getBackend();
      try {
        await edit(backend);
        report(what);
        return true;
      } catch (e) {
        refuse(e instanceof Error ? e.message : "That could not be saved.");
        return false;
      }
    },
    [report, refuse, advancedPrefs.protectLibrary],
  );
  /** Runs one edit and reports what happened, refusals included. */
  const runEdit = useCallback(
    async (what: string, edit: (b: Awaited<ReturnType<typeof getBackend>>) => Promise<unknown>) => {
      await writeEdit(what, edit);
    },
    [writeEdit],
  );

  /** Shows an edit at once, so the interface does not wait on the reload. */
  const showPending = useCallback((id: string, patch: Partial<RowDto>) => {
    setPendingEdits((edits) => new Map(edits).set(id, { ...edits.get(id), ...patch }));
  }, []);
  /** Takes back one field of edits shown at once, when the write did not happen. */
  const dropPending = useCallback((ids: readonly string[], field: keyof RowDto) => {
    setPendingEdits((edits) => {
      const next = new Map(edits);
      for (const id of ids) {
        const patch = { ...next.get(id) };
        delete patch[field];
        if (Object.keys(patch).length === 0) next.delete(id);
        else next.set(id, patch);
      }
      return next;
    });
  }, []);

  /**
   * A file the Explorer lists that the library does not hold cannot be
   * edited: the write would match no row. Said once, here, for every edit.
   */
  const refuseLoose = useCallback(
    (id: string) => {
      if (!isLooseId(id)) return false;
      refuse("That file is not in the collection. Import it first.");
      return true;
    },
    [refuse],
  );

  /** Rates one track from the list, or the information panel's whole selection. */
  const rateTracks = useCallback(
    (ids: readonly string[], stars: number) => {
      if (ids.some(refuseLoose)) return;
      for (const id of ids) showPending(id, { rating: stars });
      void writeEdit(stars === 0 ? "Rating cleared." : `Rated ${stars} of 5.`, (b) =>
        b.edits.setTrackRating(ids, stars),
      ).then((written) => {
        // A refused write takes back what was shown, so the row says what
        // the Library holds.
        if (!written) dropPending(ids, "rating");
      });
    },
    [writeEdit, showPending, dropPending, refuseLoose],
  );
  const rateTrack = useCallback((id: string, stars: number) => rateTracks([id], stars), [rateTracks]);

  const commentTracks = useCallback(
    (ids: readonly string[], comment: string) => {
      if (ids.some(refuseLoose)) return;
      for (const id of ids) showPending(id, { comment });
      void runEdit("Comment saved.", (b) => b.edits.setTrackComment(ids, comment));
    },
    [runEdit, showPending, refuseLoose],
  );
  const commentTrack = useCallback((id: string, comment: string) => commentTracks([id], comment), [commentTracks]);

  const editTrackColor = useCallback((id: string, color: string) => {
    if (refuseLoose(id)) return;
    // Let the library-changed event update the table and Info panel only after
    // the write succeeds. A refused write must leave the previous color visible.
    void runEdit(t(color === "0" ? "Color cleared." : "Color saved."), (b) =>
      b.edits.setTrackColor([id], color));
  }, [refuseLoose, runEdit, t]);

  const editTrackField = useCallback(
    (id: string, field: TrackField, value: string) => {
      if (refuseLoose(id)) return;
      // Shown before the round trip for the fields a row carries; the rest
      // belong to the information panel and arrive with the re-read.
      if (ROW_FIELDS.has(field)) showPending(id, { [field]: value });
      void runEdit(`${FIELD_LABEL[field]} saved.`, (b) =>
        b.edits.setTrackField([id], field, value, { writeKeyTag: field === "key" && analysisPrefs.writeKeyTag }),
      );
    },
    [runEdit, showPending, refuseLoose, analysisPrefs.writeKeyTag],
  );

  // Tracks a playlist already holds: left out, added again, or asked about
  // per Preferences. Null when the question was cancelled, which stops the add.
  const [duplicatePrompt, setDuplicatePrompt] = useState<{
    playlist: string; duplicates: number; total: number; settle: (answer: DuplicateAnswer | null) => void;
  } | null>(null);
  const askDuplicates = useCallback(
    async (held: number, total: number, name: string): Promise<boolean | null> => {
      if (held === 0) return false;
      const policy = advancedPrefs.duplicateTracks;
      if (policy !== "ask") return policy === "add";
      const answer = await new Promise<DuplicateAnswer | null>((settle) =>
        setDuplicatePrompt({ playlist: name, duplicates: held, total, settle }));
      setDuplicatePrompt(null);
      if (!answer) return null;
      if (answer.remember) prefs.update("advanced", { duplicateTracks: answer.choice });
      return answer.choice === "add";
    },
    [advancedPrefs.duplicateTracks, prefs],
  );
  const resolveDuplicates = useCallback(
    async (playlistId: string, trackIds: string[], name: string): Promise<boolean | null> => {
      const backend = await getBackend();
      const held = await backend.edits.playlistDuplicates(playlistId, trackIds);
      return askDuplicates(held.length, trackIds.length, name);
    },
    [askDuplicates],
  );

  /**
   * Tracks (or Explorer files, imported on the way in) added to a playlist
   * as a status-bar job, a slice at a time, so a few hundred show how far
   * along they are and can be stopped between slices. Both a drag onto a
   * playlist and the Add To Playlist menu come here (#291).
   */
  const addTracksTo = useCallback(
    (playlistId: string, ids: readonly string[]) => {
      const name = tree.find((n) => n.id === playlistId)?.name ?? t("the playlist");
      jobs.add({
        label: t("Adding {count} to {name}", { count: ids.length, name }),
        total: ids.length,
        target: playlistId,
        pendingRows: ids.length,
        run: async ({ signal, update }) => {
          const backend = await getBackend();
          // Rows out of the Explorer that the library does not hold are
          // imported on the way in, as rekordbox's Add To Playlist does with
          // them [OBS 7, Winrig 2026-10-08].
          const { ids: trackIds, report: imported } = await importLoose(ids, (paths) => backend.importPaths(paths));
          if (imported && analysisPrefs.auto && imported.tracks.length > 0) analysis.add(imported.tracks);
          const tail = skippedTail(imported?.skipped ?? []);
          if (trackIds.length === 0) {
            refuse(`Nothing added to ${name}${tail}.`);
            return;
          }
          const allowDuplicates = await resolveDuplicates(playlistId, trackIds, name);
          if (allowDuplicates === null) {
            report(`Nothing added to ${name}${tail}.`);
            return;
          }
          update({ total: trackIds.length, pendingRows: trackIds.length });
          let added = 0;
          let processed = 0;
          // Every slice of this add is one step of Edit › Undo (#499).
          const transaction = crypto.randomUUID();
          try {
            for (const slice of slices(trackIds, ADD_SLICE)) {
              if (signal.aborted) break;
              const result = await backend.edits.addTracksToPlaylist(playlistId, slice, allowDuplicates, transaction);
              added += result.added;
              processed += slice.length;
              update({ done: processed, pendingRows: trackIds.length - processed });
            }
          } finally {
            if (processed > 0) setTree(withSources(await backend.playlistTree()));
          }
          const noun = (n: number) => `${n} track${n === 1 ? "" : "s"}`;
          report(processed < trackIds.length
            ? `Stopped: added ${added} of ${noun(trackIds.length)} to ${name}${tail}.`
            : added <= 0
              ? `Already in ${name}${tail}.`
              : `Added ${noun(added)} to ${name}${tail}.`);
        },
      });
    },
    [tree, report, refuse, analysisPrefs.auto, analysis, jobs, t, resolveDuplicates],
  );

  const addDraggedTo = useCallback(
    (playlistId: string) => {
      const ids = draggedTracks?.ids;
      setDraggedTracks(null);
      if (!ids || ids.length === 0) return;
      if (advancedPrefs.protectLibrary) {
        refuse(refusal(true));
        return;
      }
      addTracksTo(playlistId, ids);
    },
    [draggedTracks, refuse, advancedPrefs.protectLibrary, addTracksTo],
  );

  // Preferences › Playlists' Ask me for a dropped folder whose name a list
  // beside it has (#355): the question for one folder at a time.
  const [folderClashPrompt, setFolderClashPrompt] = useState<{
    name: string; settle: (answer: FolderClashAnswer) => void;
  } | null>(null);
  const askFolderClash = useCallback(async (name: string): Promise<FolderClashAnswer> => {
    const answer = await new Promise<FolderClashAnswer>((settle) => setFolderClashPrompt({ name, settle }));
    setFolderClashPrompt(null);
    return answer;
  }, []);

  /**
   * Files dragged in from outside the app (Finder, Explorer) and dropped on
   * a playlist: imported, then added to that playlist, the way a dragged
   * track already is. Dropped on the Playlists root or a playlist folder,
   * each folder becomes a playlist there instead (see `dropFolders`).
   */
  const importDroppedPathsTo = useCallback(
    (playlistId: string, paths: string[]) => {
      if (advancedPrefs.protectLibrary) {
        refuse(refusal(true));
        return;
      }
      const target = tree.find((n) => n.id === playlistId);
      if (playlistId === "playlists" || target?.kind === "folder") {
        // Queued and shown in the status bar like any long import; each
        // folder is one transaction, so Stop lands between folders (#291).
        jobs.add({
          label: paths.length === 1 ? t("Importing 1 folder…") : t("Importing {count} folders…", { count: paths.length }),
          total: paths.length,
          stoppable: paths.length > 1,
          run: async ({ signal, update }) => {
            const { dropFolders } = await import("@/lib/folderDrop");
            await dropFolders(playlistId, paths, t, report, refuse, setTree, analysisPrefs.auto ? analysis.add : undefined,
              { mode: advancedPrefs.folderDropClash, ask: askFolderClash }, signal, (done) => update({ done }));
          },
        });
        return;
      }
      const name = target?.name ?? t("the playlist");
      jobs.add({
        label: t("Finding tracks for {name}", { name }),
        target: playlistId,
        run: async ({ signal, update }) => {
          const backend = await getBackend();
          // The folder is walked first, so every file is known before any is
          // imported and the bar can say how far along it is.
          const files = await backend.listImportFiles(paths);
          if (signal.aborted) return;
          if (files.length === 0) {
            refuse(`No tracks found to add to ${name}.`);
            return;
          }
          const total = files.length;
          const noun = (n: number) => `${n} track${n === 1 ? "" : "s"}`;
          // Importing is the first half of the bar and adding the second.
          update({ total: total * 2, done: 0, pendingRows: total, label: t("Importing {count} into {name}", { count: total, name }) });
          // One re-read of the library for the whole import, not one a
          // slice, and one even when a slice fails, so the slices that
          // landed are shown.
          let landed = 0;
          let sliced;
          try {
            sliced = await importWithLedger(backend, files, signal, (reached, sofar) => {
              landed = sofar.imported;
              update({ done: reached });
            });
          } finally {
            if (landed > 0) await backend.reloadLibrary();
          }
          // Files the library already held still belong in the playlist.
          const { ids: toAdd, tracks: fresh, skipped, existing } = sliced;
          update({ label: t("Adding {count} to {name}", { count: total, name }) });
          const allowDuplicates = signal.aborted ? false : await resolveDuplicates(playlistId, toAdd, name);
          let added = 0;
          let processed = 0;
          // Every slice of this add is one step of Edit › Undo (#499); the
          // import before it is not in the history.
          const transaction = crypto.randomUUID();
          for (const slice of slices(toAdd, ADD_SLICE)) {
            if (signal.aborted || allowDuplicates === null) break;
            const result = await backend.edits.addTracksToPlaylist(playlistId, slice, allowDuplicates, transaction);
            added += result.added;
            processed += slice.length;
            update({ done: total + processed, pendingRows: toAdd.length - processed });
          }
          setTree(await backend.playlistTree());
          if (analysisPrefs.auto && fresh.length > 0) analysis.add(fresh);
          const already = existing > 0 ? `; ${existing} already in the library` : "";
          const left = skippedTail(skipped);
          report(signal.aborted
            ? `Stopped: ${noun(added)} added to ${name}${left}${already}.`
            : added === 0 && toAdd.length > 0
              ? `Already in ${name}${left}${already}.`
              : `Added ${noun(added)} to ${name}${left}${already}.`);
          if (sliced.recordingWarning) refuse(sliced.recordingWarning);
        },
      });
    },
    [tree, report, refuse, advancedPrefs.protectLibrary, analysisPrefs.auto, analysis, jobs, t, resolveDuplicates,
      advancedPrefs.folderDropClash, askFolderClash],
  );

  const importDroppedPathsToCollection = useCallback((paths: string[]) => {
    if (readOnly) {
      refuse(refusal(advancedPrefs.protectLibrary));
      return;
    }
    jobs.add({
      label: t("Adding track data to the Collection"),
      run: async ({ signal, update }) => {
        const backend = await getBackend();
        const files = await backend.listImportFiles(paths);
        if (signal.aborted) return;
        if (files.length === 0) {
          refuse(t("No tracks found to import."));
          return;
        }
        update({ total: files.length });
        let landed = 0;
        let imported;
        try {
          imported = await importWithLedger(backend, files, signal, (reached, sofar) => {
            landed = sofar.imported;
            update({ done: reached });
          });
        } finally {
          if (landed > 0) await backend.reloadLibrary();
        }
        if (analysisPrefs.auto && imported.tracks.length > 0) analysis.add(imported.tracks);
        const already = imported.existing > 0 ? `; ${imported.existing} already in the library` : "";
        report(`Imported ${imported.imported} of ${files.length} files${skippedTail(imported.skipped)}${already}.`);
        if (imported.recordingWarning) refuse(imported.recordingWarning);
      },
    });
  }, [readOnly, advancedPrefs.protectLibrary, refuse, jobs, t, analysisPrefs.auto, analysis, report]);

  const importDroppedPaths = useCallback((target: string, paths: string[]) => {
    if (target === "all") importDroppedPathsToCollection(paths);
    else importDroppedPathsTo(target, paths);
  }, [importDroppedPathsToCollection, importDroppedPathsTo]);

  const importDroppedFilesTo = useCallback(
    (target: string, files: File[]) => {
      // Resolve immediately: macOS's drag pasteboard belongs to the current
      // OS drag, not to the File object retained by this callback.
      void droppedFilePaths(files)
        .then((paths) => importDroppedPaths(target, paths))
        .catch((e: unknown) => refuse(e instanceof Error ? e.message : "Those files could not be imported."));
    },
    [importDroppedPaths, refuse],
  );

  useEffect(() => subscribeNativeFileDrops((drop) => {
    const target = document.elementFromPoint(drop.x, drop.y);
    const row = target?.closest<HTMLElement>("[data-file-drop-playlist]");
    const dropTarget = row?.dataset.fileDropPlaylist
      ?? (target?.closest('[data-testid="track-scroll"]') && selectedNode?.kind === "playlist"
        ? selectedNode.id
        : target?.closest('[data-testid="track-scroll"]') && selectedNode?.kind === "allTracks"
          ? "all"
        : undefined);
    if (dropTarget) importDroppedPaths(dropTarget, drop.paths);
    else refuse(t("Drop files or folders onto All Tracks or a playlist to import them."));
  }), [selectedNode, importDroppedPaths, refuse, t]);

  /**
   * Files dropped into the open All Tracks or playlist list follow the same
   * route as their tree rows. Empty lists accept files; other sources do not.
   */
  const importDroppedFilesIntoOpen = useMemo(
    () =>
      readOnly || (selectedNode?.kind !== "playlist" && selectedNode?.kind !== "allTracks")
        ? undefined
        : (files: File[]) => importDroppedFilesTo(selectedNode.id, files),
    [readOnly, selectedNode, importDroppedFilesTo],
  );

  /**
   * What loading a track onto a deck goes through: a track whose file is
   * missing is not loaded, the deck keeps what it had, and the status bar
   * says so in rekordbox's words [OBS rekordbox 7.2.19 static:
   * `UiPlayer::handleMessageDragAndDrop` @0x101abadc4 opens only a file that
   * is there, else shows `kPlayerOperateErrorLoadMissingFile`, "Load error.
   * The file could not be found.", @0x101abb0f4 and returns].
   */
  const loadRequest = useRef({ a: 0, b: 0 });
  const loadOnto = useEventCallback(async (deck: DeckId, set: (row: RowDto | null) => void, row: RowDto | null): Promise<boolean> => {
    const request = ++loadRequest.current[deck];
    if (row && /^\d+$/.test(row.id)) {
      // A fetched collection row is only a snapshot. Its audio drive may have
      // disconnected or returned at the same path since the row was fetched.
      try {
        const backend = await getBackend();
        const missing = await backend.trackMissing(row.id);
        if (request !== loadRequest.current[deck]) return false;
        if (missing === true || (missing === null && row.missing === true)) {
          refuse(t("Load error. The file could not be found."));
          return false;
        }
      } catch (error) {
        if (request === loadRequest.current[deck]) {
          refuse(error instanceof Error ? error.message : t("Load error. The file could not be found."));
        }
        return false;
      }
    } else if (row?.missing === true) {
      // Loose/device rows are not addressed by trackMissing.
      refuse(t("Load error. The file could not be found."));
      return false;
    }
    // What this session has written to the row since the list read it, an
    // analysis among it: the library is re-read only once a run is over.
    const edits = row ? pendingEdits.get(row.id) : undefined;
    const current = row && edits ? { ...row, ...edits } : row;
    set(current);
    // A track loaded without analysis is analysed now, ahead of anything
    // waiting, and its waveform drawn on the deck from left to right as the
    // file decodes. [UNKNOWN] whether rekordbox analyses a track on load;
    // this is the behaviour the maintainer asked for. Only the library's own
    // tracks (numeric ids): a stick's or a folder's file has no row for the
    // analysis to be written to, and nothing is written to a read-only or
    // protected library.
    if (current && !(current.analysed > 0) && !readOnly && /^\d+$/.test(current.id)) {
      analysis.draw({ id: current.id, title: current.title });
    }
    return true;
  });
  const deckLoaders = useMemo(() => ({
    a: (row: RowDto | null) => { void loadOnto("a", setPlayerTrack, row); },
    b: (row: RowDto | null) => { void loadOnto("b", setPlayerTrackB, row); },
  }), [loadOnto]);
  /**
   * Where deck 1's track was opened from. Whether that list goes on into its
   * next row is asked when the track ends, as rekordbox does, since it turns
   * on the layout shown then: see `src/lib/continuousPlay.ts`. A load from
   * anywhere else leaves a stale entry behind, which the track id no longer
   * matches.
   */
  const continuation = useRef<{ position: ListPosition; trackId: string } | null>(null);
  const openInDeckA = useCallback(async (row: RowDto | null, from?: ListPosition) => {
    if (!await loadOnto("a", setPlayerTrack, row)) return false;
    continuation.current = row && from ? { position: from, trackId: row.id } : null;
    return true;
  }, [loadOnto]);
  // The tracks an analysis is drawing for a deck, so the deck shows the
  // waveform before the analysis is over.
  const drawingIds = useMemo(
    () => new Set([...analysis.state.running, ...analysis.state.pending].filter((item) => item.drawing).map((item) => item.id)),
    [analysis.state],
  );

  /**
   * Loading a dragged track into a deck.
   *
   * A pair of stable callbacks rather than one taking a deck id: `Player` is
   * memoized, and a closure built during render would give it a new prop every
   * frame the app draws.
   */
  const loadDroppedInto = useMemo(() => {
    const into = (put: (row: RowDto, position: ListPosition) => void) => () => {
      const dragged = draggedTracks;
      setDraggedTracks(null);
      // Nothing is written: which track a deck is holding is not part of the
      // library, so this is the one drop that cannot be refused.
      if (dragged) put(dragged.row, dragged.position);
    };
    return { a: into((row, position) => { void openInDeckA(row, position); }), b: into((row) => deckLoaders.b(row)) };
  }, [draggedTracks, deckLoaders, openInDeckA]);

  // Dropping a track onto a CDJ row in the LINK strip tells that player to
  // load it from us over Pro DJ Link.
  const loadDroppedOnLink = useCallback(
    (playerNumber: number) => {
      const id = draggedTracks?.row.id;
      setDraggedTracks(null);
      if (!id) return;
      void (async () => {
        const backend = await getBackend();
        try {
          await backend.loadTrackOnLink(playerNumber, id);
        } catch (e) {
          refuse(e instanceof Error ? e.message : "That track could not be sent to the player.");
        }
      })();
    },
    [draggedTracks, refuse],
  );

  /** The same three decks, loaded from the track menu or from a click. */
  const loadInto = deckLoaders;
  const loadTrack = useCallback(
    (deck: DeckId, row: RowDto) => loadInto[deck === "b" ? "b" : "a"](row),
    [loadInto],
  );
  const loadSelectedInto = useMemo(() => {
    if (!selectedRow) return { a: undefined, b: undefined };
    return {
      a: () => deckLoaders.a(selectedRow),
      b: () => deckLoaders.b(selectedRow),
    };
  }, [selectedRow, deckLoaders]);

  /**
   * Loads the row after (or before) deck 1's track in the list it came
   * from, and plays it if `play`: the end of a track (`continuePlaylist`) and
   * the track skip buttons, which rekordbox runs through the same
   * `ListViewer::loadNextTrack` @0x10040f570. A track that did not come from
   * a list has no row to move from, and nothing happens.
   */
  const loadAdjacentRow = useEventCallback(async (direction: "previous" | "next", play: boolean) => {
    const from = continuation.current;
    const loaded = playerTrack;
    if (!from || !loaded || from.trackId !== loaded.id) return;
    // rekordbox reads the next row off the list as the browser shows it now,
    // so a re-sort or a search since the load counts; a browser that has
    // moved on to another list leaves the list as it was opened.
    const shown = JSON.stringify(spec.source) === JSON.stringify(from.position.spec.source);
    const listed = shown ? spec : from.position.spec;
    try {
      const backend = await getBackend();
      const handle = await backend.openView(listed);
      if (handle.len === 0) return;
      const ids = await backend.viewIdsInRange(handle.viewId, 0, handle.len - 1);
      const remembered = JSON.stringify(listed) === JSON.stringify(from.position.spec) ? from.position.index : -1;
      const index = skipRowIndex(ids, loaded.id, remembered, direction);
      if (index < 0) return;
      const [row] = await backend.fetchRows(handle.viewId, index, 1);
      // Something else was loaded while the list was read.
      if (!row || continuation.current !== from) return;
      if (!await openInDeckA(row, { spec: listed, index }) || !play) return;
      await whenLoaded("a", row.id);
      setPlaying("a", true);
    } catch {
      // The deck says why a track would not open; a list that could not be
      // read leaves the deck where it was.
    }
  });

  /**
   * Deck 1 played its track to the end: load the next row of its list and
   * play it, where rekordbox would (`continuesFrom`). Its 2 PLAYER stops at
   * the end, and FULL BROWSER has no deck on screen here to load it.
   */
  const continuePlaylist = useEventCallback(async () => {
    const from = continuation.current;
    if (!from || !continuesFrom(from.position.spec.source, layout)) return;
    await loadAdjacentRow("next", true);
  });

  /**
   * The track skip buttons on deck 1. rekordbox loads the row with the deck's
   * play state carried over: `ListViewer::loadTrack` @0x100412500 takes it
   * from `DjEngineIF::isPlaying` for a button, where the end of a track
   * always plays [OBS rekordbox 7.2.19 static].
   */
  const skipTrackA = useCallback((direction: "previous" | "next", playing: boolean) => {
    void loadAdjacentRow(direction, playing);
  }, [loadAdjacentRow]);

  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    let before: DeckTick | null = null;
    void getBackend().then((backend) => {
      const off = backend.onDeckTick((tick) => {
        const ended = playedToEnd(before, tick.a);
        before = tick.a;
        if (ended) void continuePlaylist();
      });
      if (live) stop = off;
      else off();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, [continuePlaylist]);

  /**
   * The tree's context menu, and the track's.
   *
   * Every one of these writes, so each says why it could not rather than
   * looking as if it worked — the same rule the drop handler above follows.
   */
  const afterWrite = useCallback(
    async (said: string) => {
      const backend = await getBackend();
      // The tree now, so the node that was made is there when the note says
      // so. The generation is the backend's alone, announced as
      // `library:changed` after every write: a count bumped here as well
      // could land on the number the backend announces for the next edit,
      // and a generation that does not change is a page that is not
      // refetched — a rating lit for a moment and went out.
      setTree(withSources(await backend.playlistTree()));
      report(said);
    },
    [report],
  );

  // A write, awaited: true once it is saved and reported, false if refused.
  const writeNow = useCallback(
    async (run: (backend: Backend) => Promise<string>): Promise<boolean> => {
      const backend = await getBackend();
      try {
        const said = await run(backend);
        await afterWrite(said);
        return true;
      } catch (e) {
        refuse(e instanceof Error ? e.message : "That could not be saved.");
        return false;
      }
    },
    [afterWrite, refuse],
  );
  const write = useCallback(
    (run: (backend: Backend) => Promise<string>) => {
      void writeNow(run);
    },
    [writeNow],
  );
  // "Save as intelligent playlist" in the Track Filter bar (wbx-58): the
  // filter of All Tracks only (the backend names the open list otherwise).
  const saveFilterTo = useCallback(async (name: string, request: SaveFilterRequest): Promise<string | null> => {
    if (readOnly) return refusal(advancedPrefs.protectLibrary);
    try {
      const backend = await getBackend();
      await backend.edits.saveFilterPlaylist(name, request);
      await afterWrite(`Saved ${name}.`);
      return null;
    } catch (e) {
      return e instanceof Error ? e.message : "That could not be saved.";
    }
  }, [readOnly, advancedPrefs.protectLibrary, afterWrite]);
  const saveShellFilter = useSaveFilterPlaylist({
    filter: shellFilter.model,
    within: shellFilterBase.source.kind === "collection" ? null : (listNode?.name ?? "this list"),
    keyName: (identity) => formatKey(identity, viewPrefs.keyDisplay),
    save: saveFilterTo,
  });

  // rekordbox asks OK/Cancel ("Remove") before a track leaves a playlist, a
  // history or the Tag List, from the track menu and from the Delete key
  // [OBS static, rekordbox 7.2.19 arm64: `ListViewer::showPopupMenu`
  // @0x100407278/0x1004073a8/0x10040748c and `ListViewer::deleteKeyPressed`
  // @0x100405eb8 call `BrowseAlertWindow::showOkCancelBox` before
  // `deleteFromTagList` / `removeTrackOrderFromList`]. The message is passed
  // already translated.
  const confirmRemoval = useCallback(async (message: string): Promise<boolean> => {
    const backend = await getBackend();
    return backend.confirm(message, { yes: t("OK"), no: t("Cancel") });
  }, [t]);

  const createPlaylistIn = useCallback(
    (node: TreeNode) => {
      // rekordbox's own default name, from german.lang. It goes where the menu
      // was opened: first inside a folder, directly below a playlist.
      const parent = parentFor(tree, node);
      const index =
        node.kind === "folder" ? 0 : childrenOf(tree, parent).findIndex((n) => n.id === node.id) + 1;
      write(async (backend) => {
        await backend.edits.createPlaylist("New playlist", parent, index);
        return "Created New playlist.";
      });
    },
    [write, tree],
  );

  const createFolderIn = useCallback(
    (node: TreeNode) => {
      write(async (backend) => {
        await backend.edits.createFolder("New folder", parentFor(tree, node));
        return "Created New folder.";
      });
    },
    [write, tree],
  );

  // The intelligent playlist editor: over a new rule under a node, or over
  // an existing playlist's, read from the backend first.
  const [smartEditor, setSmartEditor] = useState<
    { mode: "create"; parent: string; name: string; rule: SmartRule } | { mode: "edit"; id: string; name: string; rule: SmartRule } | null
  >(null);
  // The My Tags a rule can name, read each time the editor opens so a tag
  // made since is there to pick.
  const [smartTags, setSmartTags] = useState<TrackLookups["myTagCategories"]>([]);
  const smartEditorOpen = smartEditor !== null;
  useEffect(() => {
    if (!smartEditorOpen) return;
    let live = true;
    void getBackend()
      .then((b) => b.trackLookups())
      .then((l) => {
        if (live) setSmartTags(l.myTagCategories);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [smartEditorOpen]);
  const createSmartPlaylistIn = useCallback(
    (node: TreeNode) => {
      setSmartEditor({
        mode: "create",
        parent: parentFor(tree, node),
        name: "Untitled Intelligent List",
        rule: { logic: "all", conditions: [] },
      });
    },
    [tree],
  );
  const editSmartPlaylist = useCallback(
    (node: TreeNode) => {
      void (async () => {
        const backend = await getBackend();
        try {
          const rule = await backend.smartRule(node.id);
          setSmartEditor({ mode: "edit", id: node.id, name: node.name, rule });
        } catch (e) {
          refuse(e instanceof Error ? e.message : "That rule could not be read.");
        }
      })();
    },
    [refuse],
  );
  const saveSmartPlaylist = useCallback(
    (name: string, rule: SmartRule) => {
      const editing = smartEditor;
      setSmartEditor(null);
      if (editing === null) return;
      write(async (backend) => {
        if (editing.mode === "create") {
          await backend.edits.createSmartPlaylist(name, editing.parent, rule);
          return `Created ${name}.`;
        }
        await backend.edits.setSmartRule(editing.id, rule);
        if (name !== editing.name) await backend.edits.renamePlaylist(editing.id, name);
        return `Saved ${name}.`;
      });
    },
    [smartEditor, write],
  );

  // Add Artwork on a playlist: pick a picture, file it with the playlist.
  const addPlaylistArtwork = useCallback(
    (node: TreeNode) => {
      void (async () => {
        const backend = await getBackend();
        const image = await backend.pickImage("Choose the artwork");
        if (image === null) return;
        write(async (b) => {
          await b.edits.addPlaylistArtwork(node.id, image);
          return `Artwork added to ${node.name}.`;
        });
      })();
    },
    [write],
  );

  // Sort Items on a folder or the Playlists heading: its children in name
  // order, one level, folders and playlists together, by the ICU collation
  // rekordbox uses for the UI's language — all decided and written by the
  // backend as one Undo step [OBS static, rekordbox 7.2.19:
  // `TreeViewer::treeMessageSortItem` @0x10156621c]. While the tree is
  // searched rekordbox refuses it with its own message (the check at
  // @0x101566324).
  const sortItems = useCallback(
    (folder: TreeNode, searching = false) => {
      if (searching) {
        refuse(t("Playlist cannot be edited or deleted while searching."));
        return;
      }
      if (readOnly) {
        explainEditLock();
        return;
      }
      const parent = folder.kind === "collection" ? TREE_ROOT : folder.id;
      if (childrenOf(tree, parent).length < 2) return;
      write(async (backend) => {
        const history = await backend.edits.sortItems(parent, viewPrefs.locale);
        setEditHistory(history);
        setLibraryEditHistory(history.undoLabel, history.redoLabel);
        return t("Sorted {name}.", { name: folder.kind === "collection" ? t("Playlists") : folder.name });
      });
    },
    [tree, write, refuse, t, viewPrefs.locale, readOnly, explainEditLock],
  );
  // Playlist › Sort Items (0x500d) from a key or the menu bar: the folder
  // selected in the tree, as rekordbox's command acts on the tree's
  // selection.
  // rekordbox runs the menu-bar and key command through the same handler as
  // the tree's menu (`TreeViewer::performCommand` @0x10157f264 ->
  // `treeMessageSortItem`), so it refuses over a searched tree as well.
  const treeSearching = useRef(false);
  const setTreeSearching = useCallback((searching: boolean) => { treeSearching.current = searching; }, []);
  useCommands({
    sortItems: () => {
      const node = selectedNode;
      if (node?.kind !== "folder" && node?.kind !== "collection") return false;
      sortItems(node, treeSearching.current);
      return true;
    },
  });

  // Add To Shortcut: the rail takes the playlist as a button of its own.
  // rekordbox 7.2.11 takes the same playlist twice; a second button that
  // opens the same thing is nothing to a user, so this keeps one.
  const addToShortcut = useCallback(
    (node: TreeNode) => {
      if (viewPrefs.shortcuts.includes(node.id)) return;
      prefs.update("view", { shortcuts: [...viewPrefs.shortcuts, node.id] });
    },
    [prefs, viewPrefs.shortcuts],
  );
  const deleteShortcut = useCallback(
    (id: string) => {
      prefs.update("view", { shortcuts: viewPrefs.shortcuts.filter((other) => other !== id) });
    },
    [prefs, viewPrefs.shortcuts],
  );
  const railShortcuts = useMemo(
    () =>
      viewPrefs.shortcuts.flatMap((id) => {
        // A playlist deleted since it was made a shortcut is not drawn; it
        // is dropped from the list the next time one is added or removed.
        const node = tree.find((n) => n.id === id);
        return node ? [{ id, name: node.name, selected: selectedNode?.id === id }] : [];
      }),
    [viewPrefs.shortcuts, tree, selectedNode],
  );
  const openShortcut = useCallback(
    (id: string) => {
      const node = tree.find((n) => n.id === id);
      if (node) setSelectedNode(node);
    },
    [tree],
  );

  const deleteNode = useCallback(
    (node: TreeNode) => {
      write(async (backend) => {
        const history = await backend.edits.deletePlaylist(node.id);
        setEditHistory(history);
        setLibraryEditHistory(history.undoLabel, history.redoLabel);
        if (selectedNode?.id === node.id) setSelectedNode(null);
        return `Deleted ${node.name}.`;
      });
    },
    [write, selectedNode],
  );

  const runLibraryHistory = useCallback(
    (action: "undo" | "redo") => {
      write(async (backend) => {
        const history = action === "undo"
          ? await backend.edits.undoEdit()
          : await backend.edits.redoEdit();
        setEditHistory(history);
        setLibraryEditHistory(history.undoLabel, history.redoLabel);
        return action === "undo" ? "Edit undone." : "Edit redone.";
      });
    },
    [write],
  );

  const renameNode = useCallback(
    (node: TreeNode, name: string) => {
      write(async (backend) => {
        await backend.edits.renamePlaylist(node.id, name);
        return `Renamed to ${name}.`;
      });
    },
    [write],
  );

  const moveNode = useCallback(
    (node: TreeNode, parent: string, index: number) => {
      write(async (backend) => {
        await backend.edits.movePlaylist(node.id, parent, index);
        return `Moved ${node.name}.`;
      });
    },
    [write],
  );

  const reorderPlaylist = useCallback(
    (playlist: string, order: readonly string[]) => {
      if (order.length === 0) return;
      write(async (backend) => {
        await backend.edits.reorderPlaylist(playlist, [...order]);
        return "Playlist reordered.";
      });
    },
    [write],
  );
  // The second list's reorder (wbx-59): by entry, so one of two entries of
  // a track moves alone. Resolves whether it was written.
  const reorderPlaylistEntries = useCallback(
    (playlist: string, entries: readonly string[]) => writeNow(async (backend) => {
      await backend.edits.reorderPlaylistEntries(playlist, [...entries]);
      return "Playlist reordered.";
    }),
    [writeNow],
  );
  const reorderPlaylistTracks = useCallback(
    (order: readonly string[]) => {
      if (spec.source.kind === "playlist") reorderPlaylist(spec.source.id, order);
    },
    [reorderPlaylist, spec.source],
  );

  /**
   * Whether the rows can be dragged into a new order.
   *
   * Only a playlist has an order of its own to change. It also has to be the
   * order on screen: sorted by a column, or narrowed by the search or the
   * filter, the rows are a rearrangement or a subset of the playlist, and
   * writing what is visible as the whole order would scramble the rest.
   */
  const canReorder =
    spec.source.kind === "playlist" &&
    !readOnly &&
    sortState.column === "trackNo" &&
    query === "" &&
    spec.filter === undefined;

  // Asked first, as rekordbox asks (its history wording). One Undo entry
  // for the whole selection; play counts stay as they are (#508).
  const removeTracksFromHistory = useCallback(
    async (history: string, ids: readonly string[]): Promise<boolean> => {
      if (ids.length === 0) return false;
      if (!(await confirmRemoval(t("Are you sure you want to remove the selected tracks?")))) return false;
      return writeNow(async (backend) => {
        await backend.edits.removeFromHistory(history, [...ids]);
        return `Removed ${ids.length} play${ids.length === 1 ? "" : "s"} from the history.`;
      });
    },
    [confirmRemoval, t, writeNow],
  );
  const removeFromHistory = useCallback(
    (ids: readonly string[]): Promise<boolean> => {
      if (spec.source.kind !== "history") return Promise.resolve(false);
      return removeTracksFromHistory(spec.source.id, ids);
    },
    [removeTracksFromHistory, spec.source],
  );

  const resetPlayCount = useCallback(
    (ids: readonly string[]) => {
      if (ids.length === 0) return;
      write(async (backend) => {
        await backend.edits.resetPlayCount([...ids]);
        return `DJ Play Count reset on ${ids.length} track${ids.length === 1 ? "" : "s"}.`;
      });
    },
    [write],
  );

  const convertMemoryCues = useCallback(
    (row: RowDto) => {
      void (async () => {
        const backend = await getBackend();
        try {
          const made = await backend.edits.convertMemoryCuesToHot(row.id);
          report(
            made === 0
              ? "No memory cue to convert, or every hot cue slot is taken."
              : `${made} memory cue${made === 1 ? "" : "s"} converted to hot cues.`,
          );
        } catch (e) {
          refuse(e instanceof Error ? e.message : "The cues could not be converted.");
        }
      })();
    },
    [report, refuse],
  );

  // Asked first, as rekordbox asks: the tracks leave every playlist as well
  // as the collection, and there is no undo in the window.
  const removeFromCollection = useCallback(
    async (ids: readonly string[]): Promise<boolean> => {
      if (ids.length === 0) return false;
      const backend = await getBackend();
      // The Mutants that would go with the tracks, named before asking
      // (wbx-80); a failure to read them is said, not hidden.
      const mutants = await backend.edits
        .collectionRemovalMutants([...ids])
        .catch((e: unknown) => ({ error: e instanceof Error ? e.message : String(e) }));
      const sure = await backend.confirm(removalQuestion(ids.length, mutants));
      if (!sure) return false;
      jobs.add({
        label: t("Removing {count} from the collection", { count: ids.length }),
        total: ids.length,
        run: async ({ signal, update }) => {
          // Each slice is one transaction: a Stop, or a slice that fails,
          // leaves the slices before it removed whole and the rest untouched.
          let removed = 0;
          try {
            await removeInSlices(ids, (slice) => backend.edits.removeFromCollection(slice), signal, (count) => {
              removed = count;
              update({ done: count });
            });
          } finally {
            if (removed > 0) setTree(withSources(await backend.playlistTree()));
          }
          const noun = `${removed} track${removed === 1 ? "" : "s"}`;
          report(removed < ids.length ? `Stopped: removed ${noun} from the collection.` : `Removed ${noun} from the collection.`);
        },
      });
      return true;
    },
    [jobs, report, t],
  );

  // A missing track's menu [OBS rekordbox 7.2.14, issue #201]. Auto
  // Relocate searches the stored Auto Relocate Search Folders (not shown in
  // Preferences; Music, Movies and Desktop by default) for each selected
  // track's file name; Relocate asks for each selected track's file
  // in turn, as rekordbox's `relocateSelectedFiles` does (src/lib/relocate.ts).
  const relocateSearch = useMemo((): RelocateSearch => ({
    folders: advancedPrefs.relocateUserFolders ? [...advancedPrefs.relocateFolders] : [],
    music: advancedPrefs.relocateMusic,
    video: advancedPrefs.relocateVideo,
    desktop: advancedPrefs.relocateDesktop,
  }), [
    advancedPrefs.relocateUserFolders, advancedPrefs.relocateFolders, advancedPrefs.relocateMusic,
    advancedPrefs.relocateVideo, advancedPrefs.relocateDesktop,
  ]);
  // [OBS rekordbox 7.2.19 static] rekordbox runs the search under a
  // "Searching..." progress window (`UpdateMissingFiles`, a
  // `ThreadWithProgressWindow` built @0x1012a760c, run from
  // `relocateSelectedFiles` @0x1012a6bd0). Here it is a status-bar job of
  // that name, so a long search is seen to be running (#286). It is one
  // backend call, so it has no Stop.
  const searching = useCallback(
    <T,>(work: () => Promise<T>): Promise<T> =>
      jobs.perform({ label: t("Searching..."), holdsAnalysis: true }, () => work()),
    [jobs, t],
  );
  const autoRelocate = useCallback(
    (ids: readonly string[]) => {
      if (ids.length === 0) return;
      write(async (b) => {
        const done = await searching(() => b.autoRelocate(relocateSearch, [...ids]));
        return autoRelocated(done, b, t);
      });
    },
    [write, searching, relocateSearch, t],
  );
  const relocate = useCallback(
    (ids: readonly string[]) => {
      if (ids.length === 0) return;
      write(async (b) => {
        await relocateTracks(missingAmong(ids, (page) => b.relocationTargets(page)), relocateSteps(b, t));
        return "";
      });
    },
    [write, t],
  );

  // Import To Collection, over the Explorer's files: their ids are their
  // paths behind `file:`.
  const importToCollection = useCallback(
    (ids: readonly string[]) => {
      const paths = ids.filter(isLooseId).map((id) => id.slice("file:".length));
      if (paths.length === 0) return;
      // [OBS rekordbox 7.2.19 static] rekordbox adds files to the
      // collection under a progress window titled "Adding track data to the
      // Collection" (english.lang). Here it is a status-bar job of that name,
      // in slices of one transaction each, so a few hundred files show how
      // far along they are and Stop keeps the slices already imported (#291).
      jobs.add({
        label: t("Adding track data to the Collection"),
        total: paths.length,
        run: async ({ signal, update }) => {
          const backend = await getBackend();
          let landed = 0;
          let imported;
          try {
            imported = await importWithLedger(backend, paths, signal, (reached, sofar) => {
              landed = sofar.imported;
              update({ done: reached });
            });
          } finally {
            // One re-read for the whole import, and one after a failed slice
            // too, so the slices that landed are shown.
            if (landed > 0) await backend.reloadLibrary();
          }
          if (analysisPrefs.auto && imported.tracks.length > 0) analysis.add(imported.tracks);
          const values = {
            imported: imported.imported, total: imported.imported + imported.skipped.length,
            existing: imported.existing, count: paths.length,
          };
          const said = signal.aborted
            ? t("Stopped: imported {imported} of {count} files.", values)
            : `Imported ${values.imported} of ${values.total} files${skippedTail(imported.skipped)}`;
          const already = imported.existing > 0 ? `; ${imported.existing} already in the library` : "";
          await afterWrite(signal.aborted ? said : `${said}${already}.`);
          if (imported.recordingWarning) refuse(imported.recordingWarning);
        },
      });
    },
    [afterWrite, analysisPrefs.auto, analysis, jobs, t, refuse],
  );

  // Analysis Lock is one atomic hand edit over the whole selection.
  const analysisLock = useCallback(
    (ids: readonly string[], on: boolean) => {
      if (ids.length === 0) return;
      void (async () => {
        const backend = await getBackend();
        try {
          await backend.edits.gridLockSelection(ids, on);
          report(`${on ? "Locked" : "Unlocked"} the analysis of ${ids.length} track${ids.length === 1 ? "" : "s"}.`);
        } catch (e) {
          refuse(e instanceof Error ? e.message : "The lock could not be set.");
        }
      })();
    },
    [report, refuse],
  );

  // Add To Playlist. Files the Explorer lists that the library does not hold
  // are imported first, as rekordbox's menu offers it over them [OBS 7,
  // Winrig 2026-10-08] and as a file dropped on a playlist already is.
  const addToPlaylist = useCallback(
    (playlist: string, ids: readonly string[]) => {
      if (ids.length === 0) return;
      addTracksTo(playlist, ids);
    },
    [addTracksTo],
  );

  const addToTagList = useCallback(
    (ids: readonly string[]) => {
      if (ids.length === 0) return;
      write(async (backend) => {
        await backend.edits.addToTagList([...ids]);
        return `Added ${ids.length} track${ids.length === 1 ? "" : "s"} to the Tag List.`;
      });
    },
    [write],
  );

  const reloadTag = useCallback(
    (ids: readonly string[]) => {
      if (ids.length === 0) return;
      write(async (backend) => {
        await backend.edits.reloadTags([...ids]);
        return `Tags reloaded on ${ids.length} track${ids.length === 1 ? "" : "s"}.`;
      });
    },
    [write],
  );

  // Asked first, as rekordbox asks. One Undo entry for the whole selection
  // (#508).
  const removeFromTagList = useCallback(
    async (ids: readonly string[]): Promise<boolean> => {
      if (ids.length === 0) return false;
      const sure = await confirmRemoval(t(
        "Are you sure you want to remove the selected track(s) from the Tag List?\nTrack(s) will be removed from the Tag Lists of all synced devices.",
      ));
      if (!sure) return false;
      return writeNow(async (backend) => {
        await backend.edits.removeFromTagList([...ids]);
        return `Removed ${ids.length} track${ids.length === 1 ? "" : "s"} from the Tag List.`;
      });
    },
    [confirmRemoval, t, writeNow],
  );

  // Export Track: onto a connected stick, in no playlist, with the DJ's
  // approval from the staged write (wbx-45.4). Returns when it is over.
  const exportTrackToNow = useCallback(
    (path: string, ids: readonly string[], approval: string): Promise<void> | undefined => {
      if (ids.length === 0) return;
      const device = devices.find((d) => d.path === path);
      const name = device?.name ?? "the device";
      setSyncing(true);
      report(`Writing ${ids.length} track${ids.length === 1 ? "" : "s"} to ${name}…`);
      // Returned so the shell's staged write knows when it is over.
      return (async () => {
        try {
          const backend = await getBackend();
          const written = await backend.exportTracksToDevice([...ids], path, stickDefaults, compatibilityFormat, approval);
          report(exportSummary(name, written, t));
          setDevices(await backend.listDevices());
        } catch (e) {
          if (e && typeof e === "object" && "kind" in e && e.kind === "cancelled" || e instanceof Error && e.message === "Export stopped.") report("Export stopped.");
          else refuse(e instanceof Error ? e.message : "That export could not be written.");
        } finally {
          setSyncing(false);
        }
      })();
    },
    [devices, report, refuse, stickDefaults, compatibilityFormat, t],
  );
  // Every Export Track (the track menus, a deck, a drop on a Stick) is
  // staged: the export's own plan, then the hold (wbx-45.4).
  const exportTrackTo = useCallback((path: string, ids: readonly string[]) => {
    if (ids.length === 0) return;
    const name = devices.find((d) => d.path === path)?.name ?? path;
    void stickGate.stage({
      path,
      title: `Export ${ids.length} track${ids.length === 1 ? "" : "s"} to ${name}`,
      lines: [],
      // Every argument the write takes, so the confirmation is for exactly it.
      plan: {
        kind: "exportTracks", tracks: [...ids],
        ...(stickDefaults ? { defaults: stickDefaults } : {}), ...(compatibilityFormat ? { compatibilityFormat } : {}),
      },
      run: (approval) => exportTrackToNow(path, ids, approval) ?? Promise.resolve(),
    });
  }, [devices, stickGate, stickDefaults, compatibilityFormat, exportTrackToNow]);

  // Tracks dropped on a stick in the Devices tree: Export Track to it, as
  // rekordbox exports what is dragged there (manual 7.2.18 p. 108).
  const exportDraggedTo = useCallback(
    (path: string) => {
      const ids = draggedTracks?.ids;
      setDraggedTracks(null);
      if (ids && !hasLooseId(ids)) exportTrackTo(path, ids);
    },
    [draggedTracks, exportTrackTo],
  );

  // Export Loop As WAV: where to, then the loop's stretch of the track.
  const exportLoop = useCallback(
    (trackId: string, title: string, inMs: number, outMs: number) => {
      void (async () => {
        const backend = await getBackend();
        try {
          const frames = await backend.exportLoopWav(trackId, title, inMs, outMs);
          if (frames === null) return;
          report(`Loop written: ${((outMs - inMs) / 1000).toFixed(2)} s of ${title}.`);
        } catch (e) {
          refuse(e instanceof Error ? e.message : "The loop could not be written.");
        }
      })();
    },
    [report, refuse],
  );

  // What the track menu's Add To Playlist and Export Track offer: the
  // playlists under their folders, and the connected sticks.
  const menuPlaylists = useMemo(() => playlistPaths(tree), [tree]);
  const menuDevices = useMemo(() => devices.map((d) => ({ id: d.path, name: d.name })), [devices]);

  // Asked first, as rekordbox asks, though Edit › Undo can bring them back.
  const removeTracksFromPlaylist = useCallback(
    async (playlist: string, ids: readonly string[]): Promise<boolean> => {
      if (ids.length === 0) return false;
      const sure = await confirmRemoval(t(
        "Are you sure you want to remove the selected track(s) from the playlist?\nTrack(s) will be removed from the playlists of all synced devices.",
      ));
      if (!sure) return false;
      return writeNow(async (backend) => {
        await backend.edits.removeTracksFromPlaylist(playlist, [...ids]);
        return `Removed ${ids.length} track${ids.length === 1 ? "" : "s"}.`;
      });
    },
    [confirmRemoval, t, writeNow],
  );
  const removeFromPlaylist = useCallback(
    (ids: readonly string[]): Promise<boolean> => {
      if (spec.source.kind !== "playlist") return Promise.resolve(false);
      return removeTracksFromPlaylist(spec.source.id, ids);
    },
    [removeTracksFromPlaylist, spec.source],
  );

  const revealTrack = useCallback((row: RowDto) => {
    void (async () => {
      const backend = await getBackend();
      try {
        await backend.revealTrack(row.id);
      } catch (e) {
        refuse(e instanceof Error ? e.message : "That file could not be shown.");
      }
    })();
  }, [refuse]);

  // Clear the note after a moment: it reports an action, not a state.
  useEffect(() => {
    if (note === null || note.busy) return;
    const timer = setTimeout(() => setNote(null), note.failed ? 10000 : 4000);
    return () => {
      clearTimeout(timer);
    };
  }, [note]);

  // Analysis writes the result to the library, so it is refused the way any
  // other write is while rekordbox holds the file or the library is protected.
  // The message names the real cause, as every other write refusal does.
  const refuseAnalysis = useEventCallback(() => refuse(refusal(advancedPrefs.protectLibrary)));
  const [analysisSelection, setAnalysisSelection] = useState<readonly QueueItem[] | null>(null);
  /** Capture either browser's selection before opening the settings dialog. */
  const analyseTracks = useEventCallback((tracks: readonly { id: string; title: string }[]) => {
    if (tracks.length === 0) return;
    if (readOnly) {
      refuseAnalysis();
      return;
    }
    setAnalysisSelection(tracks.map(({ id, title }) => ({ id, title })));
  });
  const analyseSelection = useEventCallback(() => analyseTracks(selectedTracks));
  const reportMainSelection = useCallback((tracks: { id: string; title: string }[]) => {
    setSelectedTracks(tracks);
    setInfoSelection(tracks.map((t) => t.id));
  }, []);
  /**
   * The shell's view changes. The Library list stays mounted but inert while
   * another view shows (wbx-45.2): it reports an empty selection then, and
   * its own again when shown, so what a command targets is always what the
   * list shows as selected.
   */
  const changeShellView = useEventCallback((next: ShellView) => {
    setShellView(next);
  });
  const reportSubSelection = useCallback((tracks: { id: string; title: string }[]) => {
    setInfoSelection(tracks.map((t) => t.id));
  }, []);
  /** Configure one track: the deck's own, from its menu. */
  const analyseOne = useCallback(
    (id: string, title: string) => {
      if (readOnly) {
        refuseAnalysis();
        return;
      }
      setAnalysisSelection([{ id, title }]);
    },
    [readOnly, refuseAnalysis],
  );
  // Once, at launch, with Auto Analysis on: rekordbox asks "Auto Analysis is
  // starting." before it analyses the Collection tracks it never analysed.
  const [autoAnalysis, setAutoAnalysis] = useState<UnanalysedTracks | null>(null);
  const autoAnalysisAsked = useRef(false);
  useEffect(() => {
    if (autoAnalysisAsked.current || !sessionReady || summary === null) return;
    autoAnalysisAsked.current = true;
    void getBackend()
      .then(backend => autoAnalysisOffer(backend, { auto: analysisPrefs.auto, readOnly }))
      .then(setAutoAnalysis)
      .catch(() => {
        // No prompt is the quiet outcome; analysis is still there on demand.
      });
  }, [sessionReady, summary, analysisPrefs.auto, readOnly]);
  const startAutoAnalysis = useEventCallback(async (offer: UnanalysedTracks, settings: AnalysisChoice) => {
    setAutoAnalysis(null);
    if (readOnly) {
      refuseAnalysis();
      return;
    }
    // Gathered before queueing, so stopping the run cannot be undone by a
    // page arriving after it.
    const tracks = [...offer.tracks];
    try {
      await takeRemainingPages(await getBackend(), offer.next, page => tracks.push(...page));
    } catch {
      // Analyse what was found; the rest is offered again at the next launch.
    }
    analysis.add(tracks, settings);
  });

  // Import from a picker: files (Import) or whole folders (Import Folder). Both
  // land the same way — pick, import, refresh the tree, queue Auto Analysis —
  // so the only difference is which native picker opens and the status wording.
  const runImport = useCallback(
    async (choosing: string, pick: (backend: Backend) => Promise<ImportReport | null>) => {
      report(choosing);
      try {
        const backend = await getBackend();
        const imported = await pick(backend);
        if (imported === null) {
          setNote(null);
          return;
        }
        const total = imported.imported + imported.skipped.length;
        const already = imported.existing.length > 0 ? `; ${imported.existing.length} already in the library` : "";
        report(`Imported ${imported.imported} of ${total} files${skippedTail(imported.skipped)}${already}.`);
        setTree(await backend.playlistTree());
        // Auto Analysis in Preferences: what just landed goes straight into
        // the queue, as rekordbox does unless told not to.
        if (analysisPrefs.auto && imported.tracks.length > 0) analysis.add(imported.tracks);
      } catch (e) {
        refuse(e instanceof Error ? e.message : "Those files could not be imported.");
      }
    },
    [report, refuse, analysisPrefs.auto, analysis],
  );

  const importFromMenu = useCallback(
    () => runImport("Choosing files to import…", (backend) => backend.importFiles()),
    [runImport],
  );

  const importFolderFromMenu = useCallback(
    () => runImport("Choosing a folder to import…", (backend) => backend.importFolder()),
    [runImport],
  );

  // File > Import rekordbox xml / iTunes Library, loaded on demand to keep
  // the first paint small.
  const importXmlFromMenu = useCallback(async (source: "rekordbox" | "itunes" = "rekordbox") => {
    const { importCollection } = await import("@/lib/xmlImport");
    await importCollection(source, t, setNote, async (backend, imported) => {
      setTree(await backend.playlistTree());
      if (analysisPrefs.auto && imported.tracks.length > 0) analysis.add(imported.tracks);
    });
  }, [t, analysisPrefs.auto, analysis]);

  const exportXmlFromMenu = useCallback(async () => {
    report("Choosing where to write the XML…");
    try {
      const backend = await getBackend();
      const written = await backend.exportXml();
      if (written === null) {
        setNote(null);
        return;
      }
      report(`Wrote ${written.toLocaleString()} tracks and the playlists as XML.`);
    } catch (e) {
      refuse(e instanceof Error ? e.message : "The XML could not be written.");
    }
  }, [report, refuse]);

  // A menu item, by id. The shell sends the id and nothing else; what it
  // means, and whether it is allowed right now, is decided in one place —
  // and the keyboard reaches it the same way on the platforms where the
  // webview keeps the accelerators from the native menu.
  const runMenu = useCallback((id: string) => {
    if (id === "stick-protection") {
      setStickProtectionOpen(true);
      return;
    }
    if (id === "undo" || id === "redo") {
      const action = id;
      const canRunLibrary = action === "undo" ? editHistory.canUndo : editHistory.canRedo;
      // Text fields keep WebKit's own history, and an armed grid editor owns
      // its labelled history. Otherwise the latest reversible library edit is
      // the app-level action.
      if (!isTyping(document.activeElement) && !hasEditHistory(action) && canRunLibrary) {
        runLibraryHistory(action);
        return;
      }
      runEditHistory(id);
      return;
    }
    const outcome = resolveMenu(id, readOnly, advancedPrefs.protectLibrary);
    if (!outcome) return;
    if ("refused" in outcome) {
      refuse(outcome.refused);
      return;
    }
    if (outcome.action === "import") {
      void importFromMenu();
      return;
    }
    if (outcome.action === "import-folder") {
      void importFolderFromMenu();
      return;
    }
    if (outcome.action === "import-xml") {
      void importXmlFromMenu();
      return;
    }
    if (outcome.action === "import-itunes") {
      void importXmlFromMenu("itunes");
      return;
    }
    if (outcome.action === "export-xml") {
      void exportXmlFromMenu();
      return;
    }
    if (outcome.action === "info") {
      setInfoOpen((open) => !open);
      setMyTagOpen(false);
      return;
    }
    if (outcome.action === "locate-loaded") {
      locateLoadedTrack();
      return;
    }
    if (outcome.action === "bar-command" && outcome.item) {
      // Track and Playlist items use the same command as their other entry
      // points. Sort Items is owned by App and has no browser-side scope.
      runCommand(outcome.item.action, outcome.item.scope === "global"
        ? undefined : { scope: menuBarSide(browserFocus()) });
      return;
    }
    if (outcome.action === "analyze-track") {
      // Track › Analyze Track: the same command as its key.
      runCommand("analyseSelection");
      return;
    }
    if (outcome.action === "sub") {
      setSubOpen((open) => !open);
      return;
    }
    if (outcome.action === "tempo-slider") {
      prefs.update("view", { tempoSlider: !viewPrefs.tempoSlider });
      return;
    }
    if (outcome.action.startsWith("layout-")) {
      setLayout(asLayout(outcome.action.slice("layout-".length)));
      return;
    }
    if (outcome.action === "missing") {
      setMissingFilesOpen(true);
      return;
    }
    openPreferences("view");
  }, [
    readOnly, advancedPrefs.protectLibrary, importFromMenu, importFolderFromMenu, importXmlFromMenu, exportXmlFromMenu, refuse,
    openPreferences, prefs, viewPrefs.tempoSlider,
    editHistory, runLibraryHistory,
  ]);

  // The command palette (#54): from the view shell, a part only the old
  // layout draws (the two-deck layout, the Information and Sub-Browser
  // windows, the tempo slider) switches to that layout and shows it; the
  // palette's "Use the view shell" comes back.
  const updateView = prefs.update;
  const setShell = useCallback((on: boolean) => updateView("view", { viewShell: on }), [updateView]);
  const runOldLayout = useCallback((id: string) => {
    updateView("view", { viewShell: false });
    if (id === "info") {
      setInfoOpen(true);
      setMyTagOpen(false);
      return;
    }
    if (id === "sub") {
      setSubOpen(true);
      return;
    }
    runMenu(id);
  }, [updateView, runMenu]);
  const paletteHistory = useMemo(
    () => ({ undo: editHistory.canUndo, redo: editHistory.canRedo }),
    [editHistory.canUndo, editHistory.canRedo],
  );
  const paletteLoaded = useMemo(
    () => ({ a: playerTrack !== null, b: playerTrackB !== null }),
    [playerTrack, playerTrackB],
  );

  // The Track and Playlist menus' greyed items, worked out from the focus
  // and what the lists and trees report, and sent on whenever they change.
  useEffect(() => {
    const sent: Record<string, boolean> = {};
    let queued = false;
    const push = () => {
      queued = false;
      const enabled = menuBarEnabled({
        focus: browserFocus(),
        ...menuBarState(),
        // Analyze Track is the shell's own command, not a list's.
        has: (action, side) => side === undefined ? hasCommand(action) : hasCommand(action, side),
      });
      const changed: Record<string, boolean> = {};
      for (const [id, on] of Object.entries(enabled)) {
        if (sent[id] !== on) changed[id] = on;
      }
      if (Object.keys(changed).length === 0) return;
      Object.assign(sent, changed);
      void getBackend().then((backend) => backend.setMenuEnabled(changed)).catch(() => undefined);
    };
    const schedule = () => {
      if (queued) return;
      queued = true;
      queueMicrotask(push);
    };
    const stopFollowing = followBrowserFocus(window);
    const stopFocus = onBrowserFocus(schedule);
    const stopState = onMenuBarState(schedule);
    schedule();
    return () => {
      stopFollowing();
      stopFocus();
      stopState();
    };
  }, []);

  // Native menu clicks.
  useEffect(() => {
    let stop: (() => void) | undefined;
    void (async () => {
      const backend = await getBackend();
      stop = backend.onMenu(runMenu);
    })();
    return () => stop?.();
  }, [runMenu]);

  // The keyboard: the shell's own shortcuts, and the menu accelerators the
  // webview keeps from the native menu on Windows.
  const keyOverrides = keyMap(prefs.preferences.keyboard);
  const uiScaleStop = prefs.preferences.view.uiScale;
  const updatePreferences = prefs.update;
  /** The level Mute took the master down from, while it is muted. */
  const mutedFrom = useRef<number | null>(null);
  // The shell's own commands (see `src/lib/commands.ts`): the keys below, the
  // Track menu's Analyze Track and anything else that asks run these.
  useCommands({
    // A knob reading a step: rekordbox's command + F12 and F11.
    volumeUp: () => {
      const gain = stepMasterGain(master.level, 1);
      if (gain > 0) mutedFrom.current = null;
      master.setLevel(gain);
      return true;
    },
    volumeDown: () => {
      const gain = stepMasterGain(master.level, -1);
      if (gain > 0) mutedFrom.current = null;
      master.setLevel(gain);
      return true;
    },
    // Mute remembers where the knob was, and a second press puts it back.
    mute: ({ repeat }) => {
      if (repeat) return true;
      const next = toggleMute(master.level, mutedFrom.current);
      const changed = next.gain !== master.level || next.mutedFrom !== mutedFrom.current;
      mutedFrom.current = next.mutedFrom;
      if (changed) master.setLevel(next.gain);
      return true;
    },
    focusSearch: () => {
      searchRef.current?.focus();
      searchRef.current?.select();
      return true;
    },
    // Escape in an empty box should blur rather than do nothing, so the next
    // arrow key reaches the track list.
    clearSearch: () => {
      if (query === "") searchRef.current?.blur();
      else setQuery("");
      return true;
    },
    analyseSelection: () => {
      analyseSelection();
      return true;
    },
    // The whole-interface scale: a stop at a time, held key included. The
    // stored stop is what every window draws at (`followUiScale`), so the
    // command only has to store it. At either end the key is still ours.
    uiScaleSmaller: () => {
      updatePreferences("view", { uiScale: stepUiScale(uiScaleStop, "smaller") });
      return true;
    },
    uiScaleLarger: () => {
      updatePreferences("view", { uiScale: stepUiScale(uiScaleStop, "larger") });
      return true;
    },
    uiScaleReset: () => {
      updatePreferences("view", { uiScale: stepUiScale(uiScaleStop, "reset") });
      return true;
    },
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // The native menu's accelerators, on the platforms where a keystroke
      // in the webview never reaches them (see `menuAccelerator`).
      const item = menuAccelerator(event, platform);
      if (item !== null) {
        // Let the webview keep its own text history for keyboard commands.
        if ((item === "undo" || item === "redo") && isTyping(event.target as HTMLElement | null)) return;
        event.preventDefault();
        runMenu(item);
        return;
      }
      const action = dispatch(event, platform, event.target as HTMLElement | null, keyOverrides);
      // Only the shell's own commands are run and swallowed here; movement
      // and selection live in the table and the deck keys in the decks, which
      // listen for themselves, and everything else falls through to the
      // browser and the OS.
      if (action === null || !SHELL_COMMANDS.has(action)) return;
      if (!runCommand(action, { repeat: event.repeat })) return;
      // Escape keeps its default, so whatever else it closes still closes.
      if (action !== "clearSearch") event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [platform, runMenu, keyOverrides]);

  // The Preferences window asking for what only this window holds.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let live = true;
    void getBackend().then((backend) => {
      if (!live) return;
      stop = backend.onPreferencesReset((what) => {
        if (what === "columns") {
          cols.reset();
        } else {
          setTreeWidth(clampWidth(305, bounds()));
          setSubWidth(DEFAULT_SUB_WIDTH);
          setSubTreeWidth(DEFAULT_SUB_TREE_WIDTH);
        }
      });
    });
    return () => {
      live = false;
      stop?.();
    };
  }, [cols, bounds]);

  const refreshDevices = useCallback(() => {
    void (async () => {
      const backend = await getBackend();
      setDevices(await backend.listDevices());
    })();
  }, []);

  // The DJ's explicit action in a Stick's panel, the one place the window
  // settles a Stick's unfinished write (wbx-45.5); no read ever does. The
  // Stick is read again (the shell's outline follows `revision`).
  // Staged and held for, as every write to a Stick is (wbx-45.4): settling
  // is the one write a Stick with an unfinished write takes.
  const settleStick = useCallback(async (path: string) => {
    const name = devices.find((d) => d.path === path)?.name ?? path;
    await stickGate.stage({
      path,
      title: `Finish or roll back the unfinished write on ${name}`,
      lines: [
        "The write that did not finish is completed if it can be, or undone: the Stick holds a whole export afterwards, the last one or the one that was being written.",
        "Files another program wrote since are kept.",
      ],
      plan: { kind: "settle" },
      run: (approval) => deviceLibraries.settle(path, approval),
    });
    refreshDevices();
  }, [devices, stickGate, deviceLibraries, refreshDevices]);

  const ejectDeviceFromTree = useCallback(async (node: TreeNode) => {
    const path = devicePath(node.id);
    if (!path || ejectingDeviceRef.current || syncing || exportRunning) return;
    ejectingDeviceRef.current = true;
    setEjectingDeviceId(node.id);
    try {
      const backend = await getBackend();
      await backend.ejectDevice(path);
      // Leave the device view before removing the row. This also keeps the
      // disconnect watcher from reporting an intentional eject as a loss.
      setSelectedNode((current) => current?.id === node.id
        ? tree.find((item) => item.kind === "allTracks") ?? tree[0] ?? null
        : current);
      setDevices((current) => current.filter((device) => device.path !== path));
      report(`${node.name} safely ejected.`);
    } catch (error) {
      refuse(error instanceof Error ? error.message : `${node.name} could not be ejected.`);
    } finally {
      ejectingDeviceRef.current = false;
      setEjectingDeviceId(null);
    }
  }, [syncing, exportRunning, tree, report, refuse]);

  // A stick renamed while the panel is open moves to another mount point on
  // macOS, so the node that was selected names a path that no longer
  // exists. The medium is still the same volume, so the selection follows it
  // to its new name rather than dropping back to the track list, and the
  // status bar says what happened. Checked when the window regains focus —
  // the rename happened in the Finder, so focus is the moment it can have
  // changed — and never on a timer.
  useEffect(() => {
    let live = true;
    const onFocus = () => {
      void getBackend()
        .then((backend) => backend.listDevices())
        .then((volumes) => {
          if (live) setDevices(volumes);
        })
        .catch(() => {
          // A failed listing leaves the last one standing.
        });
    };
    window.addEventListener("focus", onFocus);
    return () => {
      live = false;
      window.removeEventListener("focus", onFocus);
    };
  }, []);
  const previousDevices = useRef<readonly Device[]>([]);
  useEffect(() => {
    const before = previousDevices.current;
    previousDevices.current = devices;
    const path = selectedNode ? devicePath(selectedNode.id) : null;
    if (path === null || devices.some((device) => device.path === path)) return;
    // Only a stick this session has seen can be said to have moved or gone;
    // a device id restored from a previous run just falls back quietly.
    if (!before.some((device) => device.path === path)) return;
    const moved = renamedDevice(devices, path, before);
    if (moved) {
      setSelectedNode({ id: deviceId(moved.device), name: moved.device.name, kind: "device", depth: 0 });
      report(`${moved.oldName} is now ${moved.device.name}.`);
    } else {
      report(`${selectedNode?.name ?? "The device"} is no longer connected.`);
    }
    // Only the device list changing can move a selected device; the node
    // itself is what is being corrected here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [devices]);

  // Refresh connected devices without starting an export.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let live = true;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stop = backend.onDevicesChanged(() => {
        void (async () => {
          const after = await backend.listDevices().catch(() => null);
          if (after === null || !live) return;
          setDevices(after);
        })();
      });
    })();
    return () => {
      live = false;
      stop?.();
    };
  }, []);

  // Devices join the tree as nodes so the Devices section renders through the
  // same path as every other section, and the Explorer's folders after them.
  const explorer = useExplorer();
  // View › Layout in Preferences decides whether All Tracks heads the
  // playlists and whether the Explorer is there at all; the rail dims a
  // section with nothing in it, so a hidden Explorer reads as empty.
  const treeNodes = useMemo(
    () => [
      ...(viewPrefs.allTracks ? tree : tree.filter((node) => node.kind !== "allTracks")),
      ...deviceLibraries.nodes,
      ...(viewPrefs.explorer ? explorer.nodes : []),
    ],
    [tree, deviceLibraries.nodes, explorer.nodes, viewPrefs.allTracks, viewPrefs.explorer],
  );
  // A lazy row was opened: a stick reads its libraries, a folder on disk
  // its subfolders.
  const expandNode = useCallback(
    (node: TreeNode) => (node.kind === "device" ? deviceLibraries.expand(node) : explorer.expand(node)),
    [deviceLibraries, explorer],
  );

  // A stick's own playlists, edited one library at a time as rekordbox's
  // Devices tree does. Nothing here touches the collection.
  const deviceBusy = syncing || exportRunning || ejectingDeviceId !== null;
  // `said` is the note once it is done, from how many entries changed.
  // `approval`: the DJ's, from the staged write (wbx-45.4).
  const editDevice = useCallback(
    async (node: TreeNode, edit: DevicePlaylistEdit, said: (changed: number) => string, approval: string) => {
      const ref = parseDeviceNodeId(node.id);
      if (!ref) return;
      if (deviceBusy) {
        refuse(t("Wait for the device to finish before changing its playlists."));
        return;
      }
      try {
        const result = await deviceLibraries.edit(ref.path, ref.format, edit, approval);
        if (result.changed > 0) report(said(result.changed));
      } catch (e) {
        refuse(e instanceof Error ? e.message : t("The device library could not be changed."));
      }
    },
    [deviceLibraries, deviceBusy, report, refuse, t],
  );
  /** One edit to a Stick's playlists, staged and held for (wbx-45.4). */
  const stageDeviceEdit = useCallback(
    (node: TreeNode, edit: DevicePlaylistEdit, title: string, lines: readonly string[], said: (changed: number) => string) => {
      const ref = parseDeviceNodeId(node.id);
      if (!ref) return Promise.resolve(false);
      return stickGate.stage({
        path: ref.path, title, lines,
        plan: { kind: "playlistEdit", format: ref.format, edit },
        run: (approval) => editDevice(node, edit, said, approval),
      });
    },
    [stickGate, editDevice],
  );
  const stickName = useCallback((path: string) => devices.find((d) => d.path === path)?.name ?? path, [devices]);
  // Tracks dropped on a Stick's playlist: the reads (which tracks it holds,
  // the duplicate question) first, then the staged write, held for, before
  // anything is written (wbx-45.4). The returned promise settles when the
  // drop is over.
  const addDraggedToDeviceNow = useCallback((
    node: TreeNode,
    approve: (change: { adding: number; exportFirst: number; plan: StickWriteOperation }) => Promise<string | null>,
  ): Promise<void> | undefined => {
    const drag = draggedTracks;
    setDraggedTracks(null);
    const ref = parseDeviceNodeId(node.id);
    if (!drag || node.kind !== "devicePlaylist" || !ref) return;
    if (deviceBusy) {
      refuse(t("Wait for the device to finish before changing its playlists."));
      return;
    }
    setSyncing(true);
    return (async () => {
      try {
        const backend = await getBackend();
        if (drag.sourceKind === "folder" && drag.ids.some((id) => id.startsWith("file:"))) {
          const approval = await approve({
            adding: drag.ids.length, exportFirst: 0,
            plan: { kind: "addFiles", format: ref.format, playlist: ref.id, files: [...drag.ids] },
          });
          if (approval === null) return;
          const result = await backend.devicePlaylistAddFiles(ref.path, ref.format, ref.id, [...drag.ids], approval);
          if (result.changed > 0) report(t("Added to {name}.", { name: node.name }));
          await deviceLibraries.refresh(ref.path);
          setDevices(await backend.listDevices());
          return;
        }
        if (drag.sourceKind === "device") {
          const tracks = [...drag.ids];
          const held = await backend.devicePlaylistDuplicates(ref.path, ref.format, ref.id, tracks);
          const duplicates = await askDuplicates(held.length, tracks.length, node.name);
          if (duplicates === null) return;
          const heldSet = new Set(held);
          const adding = duplicates ? tracks : tracks.filter((track) => !heldSet.has(track));
          if (adding.length === 0) return;
          const edit = { kind: "add" as const, playlist: ref.id, tracks: adding, duplicates };
          const approval = await approve({ adding: adding.length, exportFirst: 0, plan: { kind: "playlistEdit", format: ref.format, edit } });
          if (approval === null) return;
          const result = await deviceLibraries.edit(ref.path, ref.format, edit, approval);
          if (result.changed > 0) report(t("Added to {name}.", { name: node.name }));
          return;
        }
        // Library tracks: the reads and the question, then one approved
        // write bound to these tracks in this order (wbx-45.4, third review).
        const drop = await askDevicePlaylistDrop(backend, ref.path, ref.format, ref.id, drag.ids, askDuplicates, node.name);
        if (drop === null) return;
        const approval = await approve({
          adding: drop.adding, exportFirst: drop.missing,
          plan: {
            kind: "addLibraryTracks", format: ref.format, playlist: ref.id, tracks: drop.tracks, duplicates: drop.duplicates,
            ...(stickDefaults ? { defaults: stickDefaults } : {}), ...(compatibilityFormat ? { compatibilityFormat } : {}),
          },
        });
        if (approval === null) return;
        const result = await backend.devicePlaylistAddLibraryTracks(
          ref.path, ref.format, ref.id, drop.tracks, drop.duplicates, stickDefaults, compatibilityFormat, approval,
        );
        if (result.changed > 0) report(t("Added to {name}.", { name: node.name }));
        await deviceLibraries.refresh(ref.path);
        // Export Track can add loose files before the playlist edit. Re-read
        // both the volume list and the playlist tree after the full operation.
        setDevices(await backend.listDevices());
      } catch (error) {
        refuse(error instanceof Error ? t(error.message) : t("The device library could not be changed."));
      } finally {
        setSyncing(false);
      }
    })();
  }, [draggedTracks, deviceBusy, stickDefaults, compatibilityFormat, askDuplicates, deviceLibraries, report, refuse, t]);
  const addDraggedToDevice = useCallback((node: TreeNode) => {
    const ref = parseDeviceNodeId(node.id);
    if (!ref) return;
    const library = LIBRARY_NAMES[ref.format];
    const tracks = (n: number) => `${n} track${n === 1 ? "" : "s"}`;
    void stickGate.guarded(ref.path, (approve) => addDraggedToDeviceNow(node, (change) => approve({
      title: `Add ${tracks(change.adding)} to “${node.name}” on ${stickName(ref.path)}`,
      lines: [
        `${library}: ${tracks(change.adding)} added at the end of “${node.name}”`,
        ...(change.plan.kind === "addFiles" ? [`The ${change.adding === 1 ? "file is" : "files are"} analysed on the Stick first`] : []),
        ...(change.exportFirst > 0 ? [`${tracks(change.exportFirst)} not on the Stick yet, exported first:`] : []),
      ],
      plan: change.plan,
    })) ?? Promise.resolve());
  }, [stickGate, addDraggedToDeviceNow, stickName]);
  const createOnDevice = useCallback(
    (parent: TreeNode, folder: boolean) => {
      const at = deviceParentFor(parent);
      const ref = parseDeviceNodeId(parent.id);
      if (at === null || ref === null) return;
      // rekordbox's own names for a new one [OBS 7.2.14, Winrig 2026-10-08],
      // in the interface's language as rekordbox's are.
      const name = folder ? t("Untitled Folder") : t("Untitled Playlist");
      void stageDeviceEdit(
        parent, { kind: "create", parent: at, name, folder },
        `Create a ${folder ? "folder" : "playlist"} on ${stickName(ref.path)}`,
        [`${folder ? "Folder" : "Playlist"} “${name}” created in ${LIBRARY_NAMES[ref.format]}${parent.kind === "deviceFolder" ? `, inside “${parent.name}”` : ""}`],
        () => t("Created {name}.", { name }),
      );
    },
    [stageDeviceEdit, stickName, t],
  );
  const renameOnDevice = useCallback(
    (node: TreeNode, name: string) => {
      const ref = parseDeviceNodeId(node.id);
      if (!ref) return;
      void stageDeviceEdit(
        node, { kind: "rename", id: ref.id, name },
        `Rename on ${stickName(ref.path)}`,
        [`${LIBRARY_NAMES[ref.format]}: “${node.name}” renamed “${name}”`],
        () => t("Renamed to {name}.", { name }),
      );
    },
    [stageDeviceEdit, stickName, t],
  );
  // A stick's playlists have no undo here; the staged write says so, and its
  // hold replaces rekordbox's OK/Cancel. The tracks stay on the stick.
  const deleteOnDevice = useCallback(
    (node: TreeNode) => {
      const ref = parseDeviceNodeId(node.id);
      if (!ref) return;
      const library = LIBRARY_NAMES[ref.format];
      const under = node.kind === "deviceFolder" ? stickPlaylistsUnder(shellSticks, ref) : 0;
      const what = node.kind === "deviceFolder"
        ? `Folder “${node.name}” and the ${under} playlist${under === 1 ? "" : "s"} in it deleted from ${library}`
        : `Playlist “${node.name}” (${node.childCount ?? 0} track${node.childCount === 1 ? "" : "s"}) deleted from ${library}`;
      void stageDeviceEdit(
        node, { kind: "delete", id: ref.id },
        `Delete from ${stickName(ref.path)}`,
        [what, "The tracks stay on the Stick. This cannot be undone."],
        () => t("Deleted {name}.", { name: node.name }),
      ).then((deleted) => {
        // The selection stays in the Devices tree, on the library's
        // Playlists heading, rather than leaving the section.
        if (!deleted || selectedNode?.id !== node.id) return;
        const heading = deviceNodeId({ ...ref, role: "playlists", id: "0" });
        setSelectedNode(treeNodes.find((n) => n.id === heading) ?? null);
      });
    },
    [stageDeviceEdit, stickName, shellSticks, selectedNode, treeNodes, t],
  );
  // Import Playlist over a stick's playlist: a playlist at the end of the
  // collection's top level with the stick's entries, as rekordbox's Devices
  // tree imports one. The stick is only read. A track the collection does
  // not have is handled as Preferences › Advanced says: copied to
  // PioneerDJ/Imported from Device (the default), asked about with
  // rekordbox's own question [static, rekordbox 7.2.19
  // `import_from_d_ask_file_copy` @0x1001dc94c], left out, or the import
  // is cancelled. The notes are rekordbox's own where it has one: its
  // report when the name was taken [static, rekordbox 7.2.11 macOS,
  // `importFromDeviceReportWindow` @0x10018b1ec] and its older device
  // import's refusal when no track is in the collection
  // (`importDevicePlaylistLegacy` @0x1016689bc).
  // Change… in the question picks another folder for the copies, as
  // rekordbox's does [static, 7.2.19 `importFromDeviceCopySettingWindow::
  // buttonClicked` @0x1001d802c]: the plan is asked again for that folder's
  // free space, and OK copies there.
  const [copyPrompt, setCopyPrompt] = useState<{
    plan: DevicePlaylistImportPlan;
    /** The folder picked with Change…, or null for the default. */
    chosen: string | null;
    ref: { path: string; format: DeviceFormat; id: string };
    settle: (destination: string | null | false) => void;
  } | null>(null);
  const importFromDevice = useCallback(
    (node: TreeNode) => {
      const ref = parseDeviceNodeId(node.id);
      if (!ref) return;
      // Reading the Stick's playlist would settle an unfinished write: refused
      // as a write would be (wbx-45.4).
      const stopped = stickGate.refusal(ref.path);
      if (stopped !== null) {
        refuse(stopped);
        return;
      }
      void (async () => {
        const backend = await getBackend();
        try {
          const preference = advancedPrefs.deviceImportMissing;
          const missing: DeviceImportMissing = preference === "ask" ? "copy" : preference;
          let destination: string | null = null;
          if (preference === "ask") {
            const plan = await backend.devicePlaylistImportPlan(ref.path, ref.format, ref.id);
            if (plan.files > 0) {
              const answer = await new Promise<string | null | false>((settle) =>
                setCopyPrompt({ plan, chosen: null, ref: { path: ref.path, format: ref.format, id: ref.id }, settle }));
              setCopyPrompt(null);
              if (answer === false) return;
              destination = answer;
            }
          }
          const result = await backend.devicePlaylistImport(ref.path, ref.format, ref.id, missing, destination);
          if (result.cancelled) {
            refuse(`${t("Cannot import the playlist:")} ${result.name} ${result.missing === 1
              ? t("{count} track is not in the collection.", { count: result.missing })
              : t("{count} tracks are not in the collection.", { count: result.missing })}`);
            return;
          }
          if (result.id === null) {
            refuse(`${t("Cannot import the playlist:")} ${result.name} ${t("All songs in the list do not exist in Collection or the list is empty.")}`);
            return;
          }
          const said = [
            result.renamed
              ? `${t("Playlist name was changed because there was a playlist with the same name in your collection.")} "${result.name}"`
              : t("Imported {name}.", { name: result.name }),
          ];
          if (result.copied > 0 && destination !== null) {
            said.push(result.copied === 1
              ? t("Copied {count} music file to {folder}.", { count: result.copied, folder: destination })
              : t("Copied {count} music files to {folder}.", { count: result.copied, folder: destination }));
          } else if (result.copied > 0) {
            said.push(result.copied === 1
              ? t("Copied {count} music file to PioneerDJ/Imported from Device.", { count: result.copied })
              : t("Copied {count} music files to PioneerDJ/Imported from Device.", { count: result.copied }));
          } else if (result.missing > 0) {
            said.push(result.missing === 1
              ? t("{count} track is not in the collection and was left out.", { count: result.missing })
              : t("{count} tracks are not in the collection and were left out.", { count: result.missing }));
          }
          await afterWrite(said.join(" "));
        } catch (e) {
          refuse(e instanceof Error ? t(e.message) : t("The playlist could not be imported."));
        }
      })();
    },
    [afterWrite, refuse, t, advancedPrefs.deviceImportMissing, stickGate],
  );
  // Tracks of the selected stick library, into one of its playlists or out
  // of the one open.
  const selectedDeviceRef = useMemo(
    () => (selectedNode && isDeviceLibraryKind(selectedNode.kind) ? parseDeviceNodeId(selectedNode.id) : null),
    [selectedNode],
  );
  // A stick's library open in the browser goes with the stick when it is
  // unplugged or ejected; the tree falls back as it does for the stick's row.
  // Only a stick this session has listed can be said to have gone: at
  // startup the list is still empty.
  const listedSticks = useRef<ReadonlySet<string>>(new Set());
  useEffect(() => {
    const before = listedSticks.current;
    listedSticks.current = new Set(devices.map((device) => device.path));
    if (selectedDeviceRef && before.has(selectedDeviceRef.path) && !listedSticks.current.has(selectedDeviceRef.path)) {
      setSelectedNode(tree.find((item) => item.kind === "allTracks") ?? tree[0] ?? null);
    }
    // Only the device list going is a reason; the selection is what changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [devices]);
  const deviceMenu = useMemo(() => {
    if (!selectedNode || !selectedDeviceRef) return undefined;
    const inPlaylist = selectedNode.kind === "devicePlaylist";
    return {
      playlists: devicePlaylistsOf(treeNodes, selectedDeviceRef.path, selectedDeviceRef.format),
      inPlaylist,
      // Greyed while the stick is synced, exported or ejected, as the tree's
      // own edits are, rather than refused on the click.
      busy: deviceBusy,
      // A track the playlist already holds is asked about first, as
      // rekordbox's Devices tree asks Add or Skip [OBS 7.2.14, Winrig
      // 2026-10-12, parity/issue-186 `rekordbox-11`]; the same question and
      // Preferences choice as the collection's playlists.
      onAdd: (playlist: string, ids: readonly string[]) => {
        const target = devicePlaylistsOf(treeNodes, selectedDeviceRef.path, selectedDeviceRef.format).find((p) => p.id === playlist);
        const name = target?.name ?? "";
        const tracks = [...ids];
        const ref = selectedDeviceRef;
        // The duplicate question reads the Stick, so it waits for the gate too;
        // the write itself is staged and held for (wbx-45.4).
        void stickGate.guarded(ref.path, async (approve) => {
          try {
            const backend = await getBackend();
            const held = await backend.devicePlaylistDuplicates(ref.path, ref.format, playlist, tracks);
            const duplicates = await askDuplicates(held.length, tracks.length, name);
            if (duplicates === null) return;
            const edit = { kind: "add" as const, playlist, tracks, duplicates };
            const count = duplicates ? tracks.length : tracks.length - new Set(held).size;
            const approval = await approve({
              title: `Add ${count} track${count === 1 ? "" : "s"} to “${name}” on ${stickName(ref.path)}`,
              lines: [`${LIBRARY_NAMES[ref.format]}: ${count} track${count === 1 ? "" : "s"} added at the end of “${name}”`],
              plan: { kind: "playlistEdit", format: ref.format, edit },
            });
            if (approval === null) return;
            await editDevice(selectedNode, edit, () => t("Added to {name}.", { name }), approval);
          } catch (error) {
            refuse(error instanceof Error ? error.message : t("The device library could not be changed."));
          }
        });
      },
      // rekordbox's question names no count [OBS 7.2.14, `rekordbox-13`];
      // the note after it counts the entries that went. A track listed
      // twice is one row id here, so both copies are selected and go
      // together, as in the collection's playlists.
      onRemove: (ids: readonly string[]) => {
        if (!inPlaylist || ids.length === 0) return;
        // The hold replaces rekordbox's question, in its words (wbx-45.4).
        const name = selectedNode.name;
        void stageDeviceEdit(
          selectedNode, { kind: "remove", playlist: selectedDeviceRef.id, tracks: [...ids] },
          `Remove from “${name}” on ${stickName(selectedDeviceRef.path)}`,
          [`${LIBRARY_NAMES[selectedDeviceRef.format]}: ${ids.length} entr${ids.length === 1 ? "y" : "ies"} removed from “${name}”`, t(DEVICE_ASKS.removeTracks)],
          (changed) => changed === 1 ? t("Removed {count} track.", { count: changed }) : t("Removed {count} tracks.", { count: changed }),
        );
      },
    };
  }, [selectedNode, selectedDeviceRef, treeNodes, editDevice, deviceBusy, t, askDuplicates, refuse, stickGate, stickName, stageDeviceEdit]);
  const selectedDevice = useMemo(
    () => devices.find((device) => deviceId(device) === selectedNode?.id) ?? null,
    [devices, selectedNode],
  );

  /**
   * Writes a playlist to a stick and says how it went, in the note and as
   * the returned summary; throws what went wrong, having said it. Shared by
   * the device panel and AppleScript's `export`.
   */
  const writeToDevice = useCallback(
    // `approval`: the DJ's, from the staged write (wbx-45.4).
    async (playlistId: string, device: { name: string; path: string }, approval: string): Promise<string> => {
      setSyncing(true);
      report(t("Writing to {device}…", { device: device.name }));
      try {
        const backend = await getBackend();
        const written = await backend.exportPlaylist(playlistId, device.path, stickDefaults, deleteUnlistedMusic, compatibilityFormat, approval);
        const said = exportSummary(device.name, written, t);
        report(said);
        setDevices(await backend.listDevices());
        return said;
      } catch (e) {
        const said = e && typeof e === "object" && "kind" in e && e.kind === "cancelled" || e instanceof Error && e.message === "Export stopped."
          ? "Export stopped."
          : e instanceof Error ? e.message : "That export could not be written.";
        if (said === "Export stopped.") report(said);
        else refuse(said);
        throw new Error(said, { cause: e });
      } finally {
        setSyncing(false);
      }
    },
    [report, refuse, stickDefaults, deleteUnlistedMusic, compatibilityFormat, t],
  );

  /** Export Playlist of `playlistId` to `device`, staged and held for (wbx-45.4); the summary, or null when not written. */
  const stageExportPlaylist = useCallback(
    async (playlistId: string, device: { name: string; path: string }): Promise<string | null> => {
      const node = tree.find((n) => n.id === playlistId);
      let said: string | null = null;
      await stickGate.stage({
        path: device.path,
        title: `Export the ${node?.kind === "folder" ? "folder" : "playlist"} “${node?.name ?? playlistId}” to ${device.name}`,
        lines: [],
        plan: {
          kind: "exportPlaylist", playlist: playlistId, deleteUnlistedMusic,
          ...(stickDefaults ? { defaults: stickDefaults } : {}), ...(compatibilityFormat ? { compatibilityFormat } : {}),
        },
        run: async (approval) => {
          said = await writeToDevice(playlistId, device, approval).catch(() => null);
        },
      });
      return said;
    },
    [tree, stickGate, deleteUnlistedMusic, stickDefaults, compatibilityFormat, writeToDevice],
  );

  const syncToDevice = useCallback(
    async (playlistId: string) => {
      if (!selectedDevice) return;
      // Said in the note already; nothing else to do with it here.
      await stageExportPlaylist(playlistId, selectedDevice);
    },
    [selectedDevice, stageExportPlaylist],
  );

  // AppleScript. The backend answers a script's reads itself; what it asks
  // of the window comes here and runs through the same callbacks the
  // controls use. See `src/lib/scripting.ts`.
  const scriptHandlers = useEventCallback((): Readonly<Record<string, ScriptHandler>> => ({
    "deck.load": async ({ deck, row }) => {
      const id: DeckId = deck === "b" ? "b" : "a";
      if (deckCount(layout) < deckNumber(id)) {
        throw new Error(`Deck ${deckNumber(id)} is not in this layout. Choose a layout with ${deckNumber(id)} players from the View menu.`);
      }
      const track = row as RowDto;
      loadTrack(id, track);
      await whenLoaded(id, track.id);
    },
    "deck.play": ({ deck }) => setPlaying(deck === "b" ? "b" : "a", true),
    "deck.pause": ({ deck }) => setPlaying(deck === "b" ? "b" : "a", false),
    "link.set": async ({ on }) => {
      const status = await setLinkOn(on === true);
      if (on === true && !status.on) throw new Error(status.problem ?? "LINK could not be turned on.");
    },
    "preferences.set": ({ path, value }) => {
      const next = withSetting(prefs.preferences, String(path), value);
      const [pane, field] = String(path).split(".") as [PreferencePane, string];
      prefs.update(pane, { [field]: (next[pane] as unknown as Record<string, unknown>)[field] });
      return next;
    },
    export: async ({ playlist, device }) => {
      const to = device as { name: string; path: string };
      // A script's export is staged and held for like any other (wbx-45.4).
      const said = await stageExportPlaylist(String(playlist), to);
      if (said === null) throw new Error("The export to the Stick was not confirmed, so nothing was written.");
      return said;
    },
  }));
  useEffect(() => {
    let stop: (() => void) | undefined;
    let live = true;
    void getBackend().then((backend) => {
      if (live) stop = backend.serveScripts((request) => answer(scriptHandlers(), request));
    });
    return () => {
      live = false;
      stop?.();
    };
  }, [scriptHandlers]);
  // A script reads the preferences from the backend's copy, kept current here.
  useEffect(() => {
    void getBackend().then((backend) => backend.mirrorPreferences(prefs.preferences)).catch(() => undefined);
  }, [prefs.preferences]);

  const exportPlaylistFile = useCallback((node: TreeNode, format: "m3u8" | "txt") => {
    void (async () => {
      const backend = await getBackend();
      report(`Choosing where to write ${node.name}…`);
      try {
        const written = await backend.exportPlaylistFile(node.id, node.name, format);
        if (written === null) {
          setNote(null);
          return;
        }
        report(`Wrote ${node.name} as ${format}: ${written} track${written === 1 ? "" : "s"}.`);
      } catch (e) {
        refuse(e instanceof Error ? e.message : "That file could not be written.");
      }
    })();
  }, [report, refuse]);

  // Export Playlist / Export Folder › a stick: the tree menu lists the
  // connected sticks, so the export goes straight to the one chosen.
  // Staged and held for (wbx-45.4), from the tree's menu or a drop on a Stick.
  const exportPlaylist = useCallback((node: TreeNode, path: string) => {
    const device = devices.find((d) => d.path === path);
    if (!device) {
      refuse(t("That device is no longer connected."));
      return;
    }
    void stageExportPlaylist(node.id, device);
  }, [devices, refuse, t, stageExportPlaylist]);

  // The top of the current view, kept only to write the next start's opening
  // screen. The library itself still lives entirely in Rust.
  const [screen, setScreen] = useState<{ rows: RowDto[]; count: number }>({
    rows: restored.rows,
    count: restored.count,
  });
  const onFirstRows = useCallback((rows: RowDto[], count: number) => {
    setScreen({ rows, count });
  }, []);

  useEffect(() => {
    if (sessionReady) clearWaveformPreviewCache();
  }, [sessionReady]);

  useEffect(() => {
    if (!sessionReady || screen.rows.length === 0) return;
    void getBackend().then(backend => backend.rememberScreenAssets(
      screen.rows.map(row => row.id),
      waveformKindOf(waveformPalette, false),
    ));
  }, [sessionReady, screen.rows, waveformPalette]);

  // The waveform theme in force, for the markers drawn over every waveform:
  // under Theme they get a halo (Player.module.css).
  useEffect(() => {
    document.documentElement.dataset.waveTheme = waveformPalette;
  }, [waveformPalette]);

  // Written on every change rather than on exit: a window that is force-quit,
  // or a machine that loses power, still comes back where it was. It is a few
  // hundred bytes to localStorage, not something worth batching.
  useEffect(() => {
    if (!sessionReady) return;
    saveSession({
      treeWidth,
      selectedNodeId: selectedNode?.id ?? null,
      treeExpansion,
      sort: sortState,
      infoOpen,
      myTagOpen,
      subOpen,
      filterOpen,
      // Only once the real library is up: storing the seed back over itself
      // would keep the first run's rows alive forever.
      tree: [...tree].slice(0, SEEDED_NODES),
      rows: screen.rows.slice(0, SEEDED_ROWS),
      count: screen.count,
      layout: chosenLayout,
      subWidth,
      subTreeWidth,
      trafficLight,
      artworkMode: savedArtworkMode,
      waveformZoom,
      jumpSize,
      dualControl: dual,
    });
  }, [sessionReady, treeWidth, selectedNode, treeExpansion, sortState, infoOpen, myTagOpen, subOpen, filterOpen, tree, screen, chosenLayout, subWidth, subTreeWidth, trafficLight, savedArtworkMode, waveformZoom, jumpSize, dual]);

  // The last screen, handed to the table until the backend answers. Dropped as
  // soon as the library is up, so a stale row cannot outlive its replacement —
  // and dropped when the library has failed, since a window that says the
  // library could not be opened must not go on showing last week's tracks
  // under that message (seen on a machine with no rekordbox at all).
  const seed = useMemo(
    () =>
      summary === null && loadError === null && missingLibrary === null && restored.rows.length > 0
        ? { count: restored.count, rows: restored.rows }
        : undefined,
    [summary, loadError, missingLibrary, restored.count, restored.rows],
  );

  // One place on the bar, written by whichever changed last, as in
  // rekordbox: the tracks picked in the open list, else its tally.
  const selectionText = statusBarListText(
    { picked: barShowsSelection, selected: selectedCount, totals: listTotals },
    t,
    viewPrefs.locale,
  );

  const tip = useTooltip();
  const openViewSettings = useCallback(() => openPreferences("view"), [openPreferences]);
  const ejectA = useCallback(() => { loadRequest.current.a++; setPlayerTrack(null); }, []);
  const ejectB = useCallback(() => { loadRequest.current.b++; setPlayerTrackB(null); }, []);
  const masterA = useCallback(() => setSyncMaster("a"), [setSyncMaster]);
  const masterB = useCallback(() => setSyncMaster("b"), [setSyncMaster]);
  const exportDeckTrack = useCallback((device: string, id: string) => exportTrackTo(device, [id]), [exportTrackTo]);
  const showInformation = useCallback((row: RowDto) => { loadRequest.current.a++; setPlayerTrack(row); setInfoOpen(true); setMyTagOpen(false); }, []);
  const toggleFilter = useCallback(() => setFilterOpen(was => !was), []);
  const filterBar = useMemo(() => <TrackFilter state={filterState} onChange={setFilterState}
    values={filterValues} masterBpmX100={masterBpmX100} />, [filterState, filterValues, masterBpmX100]);
  const subTree = useMemo(() => ({
    dragging: draggedTracks !== null, onDropTracks: addDraggedTo,
    canDropToDevicePlaylists: draggedTracks?.sourceKind !== "device", onDropDeviceTracks: addDraggedToDevice,
    draggingToDevice: draggedTracks !== null && !hasLooseId(draggedTracks.ids), onDropTracksOnDevice: exportDraggedTo,
    onDropFiles: readOnly ? undefined : importDroppedFilesTo,
    onExport: exportPlaylist, exportDevices: menuDevices, onExportFile: exportPlaylistFile, onCreatePlaylist: createPlaylistIn,
    onCreateFolder: createFolderIn, onDeleteNode: deleteNode, onRenameNode: renameNode,
    onMoveNode: readOnly ? undefined : moveNode, onExpand: expandNode,
    showCounts: viewPrefs.playlistCounts, onOpenSync: openSyncManager,
    onCreateSmartPlaylist: createSmartPlaylistIn, onEditSmartPlaylist: editSmartPlaylist,
    onAddArtwork: addPlaylistArtwork, onAddToShortcut: addToShortcut, onSortItems: sortItems,
    onDeviceCreate: createOnDevice, onDeviceRename: renameOnDevice,
    onDeviceDelete: deleteOnDevice, onDeviceImport: importFromDevice,
    onEjectDevice: (node: TreeNode) => { void ejectDeviceFromTree(node); },
    ejectingDeviceId, deviceBusy: syncing || exportRunning || ejectingDeviceId !== null, readOnly,
  }), [
    draggedTracks, addDraggedTo, addDraggedToDevice, exportDraggedTo, readOnly, importDroppedFilesTo, exportPlaylist, menuDevices, exportPlaylistFile,
    createPlaylistIn, createFolderIn, deleteNode, renameNode, moveNode, expandNode,
    viewPrefs.playlistCounts, openSyncManager, createSmartPlaylistIn, editSmartPlaylist,
    addPlaylistArtwork, addToShortcut, sortItems, createOnDevice, renameOnDevice, deleteOnDevice,
    importFromDevice, ejectDeviceFromTree, ejectingDeviceId, syncing,
    exportRunning,
  ]);
  const subList = useMemo(() => ({
    onDragTracks: setDraggedTracks, onDragError: refuse, players: deckCount(layout), onLoadTrack: loadTrack,
    onShowInformation: showInformation, onShowInFinder: revealTrack, onRate: rateTrack,
    onComment: commentTrack, onEditColor: editTrackColor, onResetPlayCount: resetPlayCount, onConvertMemoryCues: convertMemoryCues,
    onRemoveFromCollection: removeFromCollection, onImportToCollection: importToCollection,
    onAutoRelocate: autoRelocate, onRelocate: relocate,
    onAnalysisLock: analysisLock, onAddToPlaylist: addToPlaylist, onAddToTagList: addToTagList,
    onRemoveFromTagList: removeFromTagList, onReloadTag: reloadTag, onExportTrack: exportTrackTo,
    playlists: menuPlaylists, devices: menuDevices, onEditField: editTrackField,
    onEditBlocked: readOnly ? explainEditLock : undefined, onFocusedRow: openInDeckA,
    onSelectedRow: setSelectedRow, onSelectedTracks: reportSubSelection, pendingEdits, readOnly,
    dragging: draggedTracks !== null,
    onDropTracks: addDraggedTo, onRemoveTracksFromPlaylist: removeTracksFromPlaylist,
    onRemoveTracksFromHistory: removeTracksFromHistory, onReorderPlaylist: reorderPlaylist,
    onDropFilesIntoPlaylist: importDroppedFilesTo, onAnalyseTracks: analyseTracks,
  }), [
    layout, loadTrack, openInDeckA, showInformation, revealTrack, rateTrack, commentTrack, editTrackColor, resetPlayCount,
    convertMemoryCues, removeFromCollection, importToCollection, autoRelocate, relocate, analysisLock, addToPlaylist,
    addToTagList, removeFromTagList, reloadTag, exportTrackTo, menuPlaylists, menuDevices,
    editTrackField, readOnly, explainEditLock, pendingEdits, draggedTracks, addDraggedTo,
    removeTracksFromPlaylist, removeTracksFromHistory, reorderPlaylist, importDroppedFilesTo,
    analyseTracks, refuse, reportSubSelection,
  ]);
  // What the status bar says, and the error in red over it. Each goes in the
  // log as it appears, so the Log window still has it once the bar moves on;
  // a running job's changing progress text is left out.
  const statusActivity = note !== null && !note.failed ? note.text : "";
  const statusError = playerError ?? loadError ?? (shellOn ? null : serviceProblem) ?? [...exportJobs.values()].find(job => job.state === "failed")?.title
    ?? (note?.failed === true ? note.text : null);
  const loggedActivity = note?.busy === true ? "" : statusActivity;
  useEffect(() => {
    if (statusError) void getBackend().then(backend => backend.logStatusMessage(true, statusError)).catch(() => {});
  }, [statusError]);
  useEffect(() => {
    if (loggedActivity) void getBackend().then(backend => backend.logStatusMessage(false, loggedActivity)).catch(() => {});
  }, [loggedActivity]);
  // The parts of the window both layouts draw, each made once here: upstream's
  // layout places them in its rows below, and the view shell (ticket #50,
  // `view.viewShell`) places the same ones in its frame. They are the same
  // elements either way, so this window stays the one owner of playback, the
  // loaded track and the tree's selection.
  const titleBar = (
      <div
        className={styles.titleBar}
        data-testid="title-bar"
        onMouseDown={startWindowDrag}
        onDoubleClick={toggleWindowMaximise}
      >
        {/* What the app is costing, in the corner: its own component, so a
            reading does not re-render the window around it. */}
        <AppCost className={styles.cost} />
        <span className={styles.appName}>wreckordbox</span>
      </div>
  );
  const deckA = (
          <Player
            track={playerTrack}
            drawing={playerTrack !== null && drawingIds.has(playerTrack.id)}
            onEject={ejectA}
            onError={setPlayerError}
            onAnalyse={analyseOne}
            onExportTrack={exportDeckTrack}
            devices={menuDevices}
            onExportLoop={exportLoop}
            simple={!isFullDeck(layout)}
            compact={shellOn}
            large={shellOn && shellView === "intake"}
            dragging={draggedTracks !== null}
            onDropTrack={loadDroppedInto.a}
            onLoadSelected={loadSelectedInto.a}
            onSkipTrack={skipTrackA}
            selectedTrackId={selectedRow?.id ?? null}
            transportSlot={deckCount(layout) > 1 ? transportA : null}
            dual={deckCount(layout) > 1}
            publishZoom={deckCount(layout) > 1 ? publishZoom.a : undefined}
            onWheelZoom={deckCount(layout) > 1 ? zoomBoth : undefined}
            bars={deckCount(layout) > 1 && dual ? dualBars : waveformZoom.a}
            onBars={deckCount(layout) > 1 && dual ? setLinkedZoom : setZoomA}
            jumpSize={jumpSize.a}
            onJumpSize={deckCount(layout) > 1 && dual ? setLinkedJump : setJumpA}
            publishSync={publishSync.a}
            {...(deckCount(layout) > 1 ? { peerSync: peerSync.a } : {})}
            isMaster={syncMaster === "a"}
            onMaster={masterA}
            synced={isFollowing(sync, "a")}
            syncOn={sync.on.a}
            onStartPlaying={startedPlaying.a}
            onSyncToggle={deckCount(layout) > 1 ? toggleSync.a : undefined}
            leaderBpmX100={syncMaster === "a" ? null : leaderBpmX100}
            onPlayingBpm={reportPlayingBpm.a}
            publishGridFollow={publishGridFollow.a}
            onGridNudge={gridNudged.a}
            onKeyShift={reportKeyShift.a}
            readOnly={readOnly}
          />
  );
  const treeView = (
        <TreeView
          nodes={treeNodes}
          selectedId={selectedNode?.id ?? null}
          onSelect={setSelectedNode}
          dragging={draggedTracks !== null}
          canDropToDevicePlaylists={Boolean(draggedTracks)}
          onDropDeviceTracks={addDraggedToDevice}
          onDropTracks={addDraggedTo}
          draggingToDevice={draggedTracks !== null && !hasLooseId(draggedTracks.ids)}
          onDropTracksOnDevice={exportDraggedTo}
          onDropFiles={readOnly ? undefined : importDroppedFilesTo}
          onExport={exportPlaylist}
          exportDevices={menuDevices}
          onExportFile={exportPlaylistFile}
          onCreatePlaylist={createPlaylistIn}
          onCreateFolder={createFolderIn}
          onDeleteNode={deleteNode}
          onRenameNode={renameNode}
          onMoveNode={readOnly ? undefined : moveNode}
          readOnly={readOnly}
          onExpand={expandNode}
          initialExpansion={restored.treeExpansion}
          onExpansionChange={setTreeExpansion}
          showCounts={viewPrefs.playlistCounts}
          onOpenSync={openSyncManager}
          onCreateSmartPlaylist={createSmartPlaylistIn}
          onEditSmartPlaylist={editSmartPlaylist}
          onAddArtwork={addPlaylistArtwork}
          onAddToShortcut={addToShortcut}
          onSortItems={sortItems}
          onSearchingChange={setTreeSearching}
          railShortcuts={railShortcuts}
          onOpenShortcut={openShortcut}
          onDeleteShortcut={deleteShortcut}
          onEjectDevice={(node) => { void ejectDeviceFromTree(node); }}
          ejectingDeviceId={ejectingDeviceId}
          deviceBusy={deviceBusy}
          onDeviceCreate={createOnDevice}
          onDeviceRename={renameOnDevice}
          onDeviceDelete={deleteOnDevice}
          onDeviceImport={importFromDevice}
        />
  );
  // The main list's columns: the Library view's own in the shell (#55).
  const listColumns = shellOn ? libraryColumns : {
    columns: cols.columns, onColumnMove: cols.move, onColumnResize: cols.resize, onColumnToggle: cols.toggle,
    onColumnAutoSize: cols.autoSize, onColumnAutoSizeAll: cols.autoSizeEvery,
    artworkMode, onArtworkMode: setArtworkMode,
  };
  // The Intake view (wbx-62): a queued track replaces deck 1's, and the
  // answer says whether it loaded; a refresh that failed is said in the
  // status line, and cleared there when it recovers, if it is still ours.
  const loadFromIntake = useCallback((row: RowDto) => openInDeckA(row), [openInDeckA]);
  // The Review view's Listen (wbx-74): load, then play once the deck is ready.
  const listenFromReview = useCallback((row: RowDto) => {
    void (async () => {
      if (!await openInDeckA(row)) return;
      const until = Date.now() + 5000;
      const tryPlay = () => {
        if (runCommand("deckPlayIfStopped", { scope: "a" }) || Date.now() > until) return;
        setTimeout(tryPlay, 50);
      };
      setTimeout(tryPlay, 0);
    })();
  }, [openInDeckA]);
  const intakeProblem = useRef<string | null>(null);
  const reportIntakeProblem = useCallback((message: string | null) => {
    const before = intakeProblem.current;
    if (message === before) return;
    intakeProblem.current = message;
    if (message !== null) setNote({ text: message, failed: true });
    else if (before !== null) setNote((current) => (current?.text === before ? null : current));
  }, []);
  // The Library view's second list (wbx-59): its state, keys and edits.
  const targetList = useTargetList({
    active: shellOn && shellView === "library", platform, tree, source: selectedNode, sourceKind: spec.source.kind,
    onOpen: setSelectedNode, selection: selectedTracks, locked: readOnly, explainLock: explainEditLock, refuse,
    add: addToPlaylist, reorderEntries: reorderPlaylistEntries, keyOverrides,
    dragged: draggedTracks, endDrag: () => setDraggedTracks(null),
    keyDisplay: viewPrefs.keyDisplay, loadedKey: trafficKey || null,
  });
  // The Library's columns at the shell's width, and beside the second list (wbx-60).
  const shellWidth = useShellWidth();
  const libraryLook = libraryLookFor(libraryWidthOf(shellWidth), targetList.shown);
  const shellTree = useShellTree({
    library: tree, sticks: shellSticksShown, selectedNode, onSelectNode: setSelectedNode,
    view: shellView, onViewChange: changeShellView, queueCounts: intakeCounts, targetId: targetList.targetId,
    // The old tree's handlers (wbx-45.4); a Stick's own tracks may go on its playlists, as in the main tree.
    actions: subTree, fromStick: draggedTracks !== null, expansion: treeExpansion, onExpansionChange: setTreeExpansion,
    stickGate, confirmDelete: confirmRemoval,
  });
  // The open list's folders and name, its heading at half width (wbx-45.21).
  const libraryCrumb = useMemo(() => breadcrumbOf(tree, selectedNode), [tree, selectedNode]);
  const library = (
        selectedDevice ? (
          <Suspense fallback={null}><DevicePanel
            device={selectedDevice}
            playlists={tree}
            onSync={syncToDevice}
            onRefresh={refreshDevices}
            onError={refuse}
            onSettle={settleStick}
            busy={syncing}
          /></Suspense>
        ) : (
        <TrackTable
          // The cached screen may mount before the backend exists. Remount
          // once restoration completes so a cancelled bootstrap view cannot
          // leave the real source permanently in its loading state.
          key={sessionReady ? "library-ready" : "library-loading"}
          spec={spec}
          deckTrackIds={deckTrackIds}
          onShowCollection={showCollection}
          onSortChange={handleSort}
          onSelectionChange={changeSelectedCount}
          onTotalsChange={setListTotals}
          onSelectedTracks={reportMainSelection}
          onAnalyse={analyseSelection}
          onShowInformation={showInformation}
          onShowInFinder={revealTrack}
          onRemoveFromPlaylist={removeFromPlaylist}
          onRemoveFromHistory={removeFromHistory}
          onResetPlayCount={resetPlayCount}
          onConvertMemoryCues={convertMemoryCues}
          onRemoveFromCollection={removeFromCollection}
          onAutoRelocate={autoRelocate}
          onRelocate={relocate}
          onImportToCollection={importToCollection}
          onAnalysisLock={analysisLock}
          onAddToPlaylist={addToPlaylist}
          onAddToTagList={addToTagList}
          onRemoveFromTagList={removeFromTagList}
          onReloadTag={reloadTag}
          onExportTrack={exportTrackTo}
          playlists={menuPlaylists}
          devices={menuDevices}
          deviceMenu={deviceMenu}
          readOnly={readOnly}
          trafficLight={activeTrafficLight}
          onTrafficLight={setTrafficLight}
          trafficKey={trafficKey}
          {...listColumns}
          {...(shellOn ? { look: libraryLook, inert: shellView !== "library" } : {})}
          onFocusedRow={(row, from) => { void openInDeckA(row, from); }}
          onSelectedRow={setSelectedRow}
          onDragTracks={setDraggedTracks}
          dragging={draggedTracks !== null}
          onDropTracks={selectedNode?.kind === "playlist" ? addDraggedTo : undefined}
          onDragError={refuse}
          onDropFiles={importDroppedFilesIntoOpen}
          players={deckCount(layout)}
          onLoadTrack={loadTrack}
          onRate={rateTrack}
          onComment={commentTrack}
          onEditColor={editTrackColor}
          // The sub-browser's list is left out on purpose: it has a source and
          // a sort of its own, which this gate does not describe.
          onReorder={canReorder ? reorderPlaylistTracks : undefined}
          // An intelligent playlist's order is its rule's, not a track number.
          onRenumber={selectedNode?.kind === "playlist" ? reorderPlaylistTracks : undefined}
          onEditField={editTrackField}
          onEditBlocked={readOnly ? explainEditLock : undefined}
          libraryGeneration={libraryGeneration}
          pendingEdits={pendingEdits}
          pendingRows={jobList
            .filter((job) => job.target !== null && job.target === selectedNode?.id)
            .reduce((sum, job) => sum + job.pendingRows, 0)}
          title={selectedNode?.name ?? "Collection"}
          query={query}
          searchField={searchField}
          onSearchFieldChange={setSearchField}
          onQueryChange={setQuery}
          searchRef={searchRef}
          seed={seed}
          onFirstRows={onFirstRows}
          filterOpen={shellOn || filterOpen}
          {...(shellOn ? { onHiddenSelectionDropped: setFilterDeselected } : { onToggleFilter: toggleFilter })}
          filterBar={shellOn ? (
            <TrackFilterBar model={shellFilter.model} onChange={onShellFilterChange} values={shellFilter.values}
              valuesError={shellFilter.valuesError} categories={shellFilter.categories} dropped={shellFilter.dropped}
              deselected={filterDeselected} onStick={shellFilter.onStick} count={shellFilter.count} filtering={shellFilter.filtering} loaded={shellFilterLoaded}
              keyDisplay={viewPrefs.keyDisplay} keySort={viewPrefs.keySort} save={saveShellFilter} />
          ) : filterBar}
        />
        )
  );
  const dialogs = (
      <Suspense fallback={null}>
      {/* Every staged Stick write, from any part of the window (wbx-45.4). */}
      {stickGate.element}
      {stickProtectionOpen ? (
        <StickProtectionDialog devices={devices} protection={stickProtection} onClose={() => { setStickProtectionOpen(false); }} />
      ) : null}
      {/* The command palette (#54): in both layouts, since Linux has no menu row. */}
      <CommandPaletteHost platform={platform} keyOverrides={keyOverrides} shell={shellOn} readOnly={readOnly}
        history={paletteHistory} loaded={paletteLoaded}
        runMenu={runMenu} runOldLayout={runOldLayout} setShell={setShell} openPreferences={openPreferences} />
      {duplicatePrompt ? (
        <DuplicateTracksDialog playlist={duplicatePrompt.playlist} duplicates={duplicatePrompt.duplicates}
          total={duplicatePrompt.total} onAnswer={duplicatePrompt.settle} />
      ) : null}
      {folderClashPrompt ? <FolderClashDialog name={folderClashPrompt.name} onAnswer={folderClashPrompt.settle} /> : null}
      {copyPrompt ? (
        <DeviceImportCopyDialog
          plan={copyPrompt.plan}
          onAnswer={(copy) => copyPrompt.settle(copy ? copyPrompt.chosen : false)}
          onChange={() => {
            void (async () => {
              const backend = await getBackend();
              const picked = await backend.pickFolder(t("Music file copy destination:"));
              if (!picked) return;
              try {
                const { path, format, id } = copyPrompt.ref;
                const plan = await backend.devicePlaylistImportPlan(path, format, id, picked);
                setCopyPrompt((open) => (open === null ? open : { ...open, plan, chosen: picked }));
              } catch (e) {
                refuse(e instanceof Error ? t(e.message) : t("That folder could not be found."));
              }
            })();
          }}
        />
      ) : null}
      {missingLibrary !== null ? (
        <NewLibraryDialog key={missingLibrary.kind} problem={missingLibrary}
          onCreate={async () => {
            await (await getBackend()).createLibrary();
            // The ready event that follows loads it like any other start.
            setMissingLibrary(null);
          }}
          // Closed by the ready event when the default folder has a library,
          // or asked again by the problem event when it is empty.
          onUseDefault={async () => (await getBackend()).useDefaultLibrary()}
          onConfirm={async (message, labels) => (await getBackend()).confirm(message, labels)}
          onQuit={() => { void getBackend().then(backend => backend.closeWindow()); }} />
      ) : null}
      {missingFilesOpen ? (
        <MissingFileManager
          readOnly={readOnly}
          search={relocateSearch}
          searching={searching}
          onWrote={(said) => { void afterWrite(said); }}
          onFailed={refuse}
          onClose={() => setMissingFilesOpen(false)}
        />
      ) : null}
      {analysisSelection !== null ? (
        <AnalysisDialog count={analysisSelection.length} initialMode={analysisPrefs.mode}
          initialBpmRange={analysisPrefs.bpmRange}
          initialFirstBeatCue={analysisPrefs.firstBeatCue}
          initialKey={analysisPrefs.detectKey}
          onCancel={() => setAnalysisSelection(null)}
          onConfirm={settings => {
            if (readOnly) { refuseAnalysis(); return; }
            analysis.add(analysisSelection, settings);
            setAnalysisSelection(null);
          }} />
      ) : null}
      {autoAnalysis !== null ? (
        <AnalysisDialog auto count={autoAnalysis.tracks.length} initialMode={analysisPrefs.mode}
          initialBpmRange={analysisPrefs.bpmRange}
          initialFirstBeatCue={analysisPrefs.firstBeatCue}
          initialKey={analysisPrefs.detectKey}
          onCancel={() => setAutoAnalysis(null)}
          onConfirm={settings => void startAutoAnalysis(autoAnalysis, settings)} />
      ) : null}
      {/* Before Preferences, so the import preferences it opens are drawn over it. */}
      {syncOpen ? (
        <SyncManager onClose={() => setSyncOpen(false)} onSynced={refreshDevices} onOpenPreferences={setSettingsOpen} />
      ) : null}
      {settingsOpen !== null ? (
        <ConnectedPreferences
          summary={summary}
          limiter={limiter.limiter}
          onLimiterChange={limiter.set}
          initialPane={settingsOpen}
          onResetColumns={cols.reset}
          onResetLayout={() => {
            setTreeWidth(clampWidth(305, bounds()));
            setSubWidth(DEFAULT_SUB_WIDTH);
            setSubTreeWidth(DEFAULT_SUB_TREE_WIDTH);
          }}
          onClose={() => setSettingsOpen(null)}
        />
      ) : null}
      {logOpen ? <LogView onClose={() => setLogOpen(false)} /> : null}
      {smartEditor ? (
        <SmartPlaylistEditor
          title={smartEditor.mode === "create" ? "Create New Intelligent Playlist" : "Edit the Intelligent Playlist"}
          name={smartEditor.name}
          rule={smartEditor.rule}
          myTags={smartTags}
          onSave={saveSmartPlaylist}
          onCancel={() => setSmartEditor(null)}
        />
      ) : null}

      </Suspense>
  );
  const linkStrip = (
      <div className={styles.linkStrip}>
        <LinkDeckStrip
          peers={linkPeers}
          link={link}
          busy={linkBusy}
          onToggle={toggleLink}
          dragging={draggedTracks !== null}
          onDropToPlayer={loadDroppedOnLink}
          onSetMaster={setLinkMaster}
          onNudgeMaster={nudgeLinkMaster}
          onTakeMasterTempo={takeLinkMasterTempo}
        />
      </div>
  );
  const Status = shellOn ? StatusLine : StatusBar;
  const statusBar = (
      <Status
        jobs={jobList}
        onStopJob={stopJob}
        exports={(exportRunning ? exportBatch : []).map(job => ({
          ...job, name: devices.find(device => device.path === job.path)?.name ?? job.path.split(/[\\/]/).filter(Boolean).at(-1) ?? job.path,
        }))}
        onOpenLog={openLog}
        onShowLog={openLogWindow}
        backupActivity={backupJob.error || backupJob.text}
        backupProgress={backupJob.progress.running ? backupJob.progress : undefined}
        version={version}
        analysisProgress={analysis.running ? {
          completed: analysis.state.done + analysis.state.failed.length,
          total: analysis.total,
          waiting: analysis.waiting,
        } : undefined}
        activity={statusActivity || (summary ? "" : "Loading the library…")}
        // Everything that went wrong, in one place and in red: the deck's
        // refusals, a library that would not open, and a write the library
        // turned down.
        error={statusError}
        onOpenProtection={!playerError && !loadError && note?.failed && note.text === refusal(true) && advancedPrefs.protectLibrary
          ? () => openPreferences("libraryProtection") : undefined}
        onCancelAnalysis={analysis.running ? analysis.cancel : undefined}
        analysisFailures={analysis.state.failed.length}
        sticks={devices.flatMap((device) => {
          const found = stickProtection.get(device.path);
          return found === undefined ? [] : [{ path: device.path, name: device.name, protected: found.protected }];
        })}
        onStickProtection={() => { setStickProtectionOpen(true); }}
        analysisFailed={analysis.state.failed}
        selection={selectionText}
        readOnly={readOnly}
        protectedLibrary={advancedPrefs.protectLibrary}
        onExplainReadOnly={() => { setPlayerError(null); explainEditLock(); }}
        onDisableReadOnly={() => {
          if (advancedPrefs.protectLibrary) {
            explainEditLock();
            return;
          }
          void getBackend().then(backend => backend.disableReadOnly()).then(() => {
            setSummary(current => current ? { ...current, readOnly: false } : current);
            setNote({ text: "Read-only mode disabled for this session.", failed: false });
          }).catch(error => refuse(error instanceof Error ? error.message : String(error)));
        }}
      />
  );
  if (shellOn) {
    return (
      <PreferencesProvider value={prefs}><StickWriteGateProvider value={stickGate}>
      <MasterOutputConnection mode={viewPrefs.vuMeter} />
      <ConfirmHost />
      <div className={styles.shellWindow} data-platform={platform.linux ? "linux" : platform.mac ? "mac" : "windows"}>
        {titleBar}
        <ViewShell
          view={shellView}
          onViewChange={changeShellView}
          platform={platform}
          keyOverrides={keyOverrides}
          player={deckA}
          tree={shellTree.tree}
          intakeQueue={shellTree.intakeQueue}
          tabCounts={{ intake: intakeCounts?.intake ?? null, review: reviewCount }}
          review={<ReviewOnDemand.View onListen={listenFromReview} loadedTrackId={playerTrack?.id ?? null} />}
          intake={(queue) => (
            <IntakeOnDemand.View
              queue={queue} onQueue={shellTree.showQueue} counts={intakeCounts}
              loadedTrackId={playerTrack?.id ?? null} onLoad={loadFromIntake} onProblem={reportIntakeProblem}
              onEdit={writeEdit} protectLibrary={advancedPrefs.protectLibrary} platform={platform} keyOverrides={keyOverrides}
            />
          )}
          stick={<StickLoadOnDemand.View devices={devices} selected={selectedDevice?.path ?? null} library={tree} revision={deviceLibraries.revision} panel={selectedDevice ? library : undefined} onEjected={refreshDevices} />}
          stickHoldsLibrary={selectedDevice !== null}
          library={library}
          libraryTarget={targetList.pane}
          libraryCrumb={libraryCrumb}
          libraryTally={crumbTally(listTotals)}
          status={<>{linkStrip}{statusBar}</>}
          topBarEnd={<><WaveformThemeChooser /><CommandPaletteButton platform={platform} /></>}
        />
        {dialogs}
      </div>
      </StickWriteGateProvider></PreferencesProvider>
    );
  }
  return (
    <PreferencesProvider value={prefs}><StickWriteGateProvider value={stickGate}>
    <MasterOutputConnection mode={viewPrefs.vuMeter} />
    <ConfirmHost />
    <div className={styles.window} data-platform={platform.linux ? "linux" : platform.mac ? "mac" : "windows"}>
      {titleBar}
      <ConnectedTopBar
        clock={clock}
        onOpenSettings={openViewSettings}
        layout={chosenLayout}
        onLayoutChange={setLayout}
        beforeSettings={<CommandPaletteButton platform={platform} />}
      />
      {/* Full Browser draws no deck at all, and no gutter under one. */}
      {deckCount(layout) > 0 ? (
        <div
          className={styles.decks}
          data-mixer={deckCount(layout) > 1 ? "" : undefined}
        >
          {/* One transport column for the pair, as rekordbox draws it: deck A
              down from the top, deck B up from the bottom, and the mixer strip
              beside it. Each deck still owns its own transport — it is
              portalled into the half of the column that belongs to it. */}
          {deckCount(layout) > 1 ? (
            <div className={styles.deckRail}>
              <div className={styles.transportSlot} ref={setTransportA} />
              {/* DUAL CONTROL, on the centre line where the capture has it:
                  it links what the two decks are showing rather than what
                  they are playing, so it belongs between them. */}
              <button
                type="button"
                className={styles.dual}
                aria-label="Dual control"
                aria-pressed={dual}
                title={tip("Link the waveform controls and beat jump across both decks.")}
                data-on={dual || undefined}
                onClick={() => setDual((was) => !was)}
              >
                <LayoutDualIcon className={styles.dualGlyph} />
              </button>
              <div className={styles.transportSlot} ref={setTransportB} />
            </div>
          ) : null}
          {/* Only with two decks. The one-player layout has no mixer and no
              crossfader, which is what rekordbox does — and what keeps a deck
              nobody has touched a fader for playing at the level of its file. */}
          {deckCount(layout) > 1 ? <MixerStrip /> : null}
          {deckA}
          {deckCount(layout) > 1 ? (
            <Player
              deck="b"
              track={playerTrackB}
              drawing={playerTrackB !== null && drawingIds.has(playerTrackB.id)}
              onEject={ejectB}
              onAnalyse={analyseOne}
              onExportTrack={exportDeckTrack}
              devices={menuDevices}
              onExportLoop={exportLoop}
              onError={setPlayerError}
              simple={!isFullDeck(layout)}
              dragging={draggedTracks !== null}
              onDropTrack={loadDroppedInto.b}
              onLoadSelected={loadSelectedInto.b}
              selectedTrackId={selectedRow?.id ?? null}
              transportSlot={transportB}
              flipped
              dual
              publishZoom={publishZoom.b}
              onWheelZoom={zoomBoth}
              bars={dual ? dualBars : waveformZoom.b}
              onBars={dual ? setLinkedZoom : setZoomB}
              jumpSize={dual ? jumpSize.a : jumpSize.b}
              onJumpSize={dual ? setLinkedJump : setJumpB}
              publishSync={publishSync.b}
              peerSync={peerSync.b}
              isMaster={syncMaster === "b"}
              onMaster={masterB}
              synced={isFollowing(sync, "b")}
              syncOn={sync.on.b}
              onStartPlaying={startedPlaying.b}
              onSyncToggle={toggleSync.b}
              leaderBpmX100={syncMaster === "b" ? null : leaderBpmX100}
              onPlayingBpm={reportPlayingBpm.b}
              publishGridFollow={publishGridFollow.b}
              onGridNudge={gridNudged.b}
              onKeyShift={reportKeyShift.b}
              readOnly={readOnly}
            />
          ) : null}
          {/* Over the line between the decks, where the capture floats it.
              After the decks, so it paints over both. */}
          {deckCount(layout) > 1 ? (
            <div className={styles.zoomSlot}>
              <DualZoom onZoom={zoomBoth} onReset={resetZoom} />
            </div>
          ) : null}
          <div className={styles.playerGutter} aria-hidden />
        </div>
      ) : null}
      <div
        data-testid="body"
        className={styles.body}
        ref={bodyRef}
        style={{ ["--tree-w" as string]: `${treeWidth}px` }}
        data-info={infoOpen || myTagOpen ? "" : undefined}
        data-mytag={myTagOpen ? "" : undefined}
        data-sub={subOpen ? "" : undefined}
      >
        {treeView}
        <div
          className={styles.splitter}
          onPointerDown={onSplitterDown}
          onPointerMove={onSplitterMove}
          onPointerUp={onSplitterUp}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize the library tree"
          aria-valuenow={treeWidth}
        />
        {library}
        {subOpen ? (
          <SubBrowser
            nodes={treeNodes}
            libraryGeneration={libraryGeneration}
            width={subWidth}
            treeWidth={subTreeWidth}
            onWidthChange={setSubWidth}
            onTreeWidthChange={setSubTreeWidth}
            // Its tree and list take part in everything the main pair does:
            // a drag from either list lands on either tree, and its rows
            // load decks and take edits the same way.
            tree={subTree}
            list={subList}
            artworkMode={subArtworkMode}
            onArtworkMode={setSubArtworkMode}
          />
        ) : null}
        {infoOpen ? (
          <Suspense fallback={null}><InfoPanel
            // The browser's selection, as rekordbox's Information Window
            // follows it; the deck's track only when nothing is selected —
            // never for a multiple selection, which the panel shows as one.
            track={infoSelection.length > 1 ? null : selectedRow ?? playerTrack}
            selection={infoSelection}
            readOnly={readOnly}
            libraryGeneration={libraryGeneration}
            onRate={rateTracks}
            onComment={commentTracks}
            onEdit={runEdit}
            onOpenPlaylist={(id) => {
              const node = tree.find((n) => n.id === id);
              if (node) setSelectedNode(node);
            }}
          /></Suspense>
        ) : null}
        {myTagOpen ? (
          <MyTagPanel
            selection={infoSelection}
            readOnly={readOnly}
            libraryGeneration={libraryGeneration}
            onEdit={runEdit}
          />
        ) : null}
        <RightRail
          className={styles.rightRail}
          myTagOpen={myTagOpen}
          onToggleMyTag={() => { setMyTagOpen((open) => !open); setInfoOpen(false); }}
          infoOpen={infoOpen}
          onToggleInfo={() => { setInfoOpen((open) => !open); setMyTagOpen(false); }}
          subOpen={subOpen}
          onToggleSub={() => setSubOpen((open) => !open)}
        />
      </div>
      {dialogs}
      {/* The LINK strip: present from the moment a player or mixer is heard,
          and the whole LINK interface from then on. It draws nothing at all
          before that, so the row it sits in collapses. */}
      {linkStrip}

      {statusBar}
    </div>
    </StickWriteGateProvider></PreferencesProvider>
  );
}
