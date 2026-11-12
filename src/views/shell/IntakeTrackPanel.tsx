/**
 * The Intake view's track panel (wbx-63): the chosen track's title and
 * artist edited in place, Energy 1 to 5, the Prep colour as any of the
 * DJ's labels, and the row of actions: "Mark Clean and next", mark Problem,
 * skip, beside the line saying what is still empty. Nothing here blocks:
 * the DJ works in any order and decides what "done" means.
 *
 * The panel holds no Library state of its own. Every value it shows is the
 * queue's latest read (re-read after every change, so an edit made by the
 * CLI or an agent shows at once), except while the DJ is typing in a field
 * (the draft is theirs until committed) and between a commit and the read
 * that carries it (shown as pending; a refused write drops it again).
 *
 * The edits themselves are `IntakeView`'s (`onTitle` and so on): they go
 * through the window's single edit path and Library Protection.
 */
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { useTranslation } from "@/i18n";
import type { IntakeColours, PrepColour } from "@/lib/intakeEdits";
import { needsLine, type QueuedTrack } from "@/lib/intakeQueue";
import { describeChord, type KeyChord, type Platform } from "@/lib/shortcuts";
import styles from "./IntakeTrackPanel.module.css";

export interface IntakeTrackPanelProps {
  track: QueuedTrack;
  /** The track's Energy as the Library holds it (0: not set). */
  energy: number;
  /** The track's Prep colour as its queue says: none in Intake, Problem in Problems. */
  prepColour: PrepColour | null;
  colours: IntakeColours | null;
  coloursFailed: string | null;
  onRetryColours: () => void;
  /** Each resolves whether the edit was written. */
  onTitle: (value: string) => Promise<boolean>;
  onArtist: (value: string) => Promise<boolean>;
  onEnergy: (level: number) => Promise<boolean>;
  onPrepColour: (colour: PrepColour | null) => void;
  onMarkClean: () => void;
  /** Ctrl+Enter in a field: its save, then (only if it saved, and still on this track) the mark. */
  onSaveThenMark: (save: () => Promise<boolean>) => void;
  onMarkProblem: () => void;
  onSkip: () => void;
  /** An edit is being written: the actions wait for it. */
  busy: boolean;
  /** Why the last edit or move did not happen; `null` when it did. */
  failure: string | null;
  /** A note that is not a failure ("That was the last track in the queue."). */
  note: string | null;
  /** The keys, as the palette shows them. */
  keys: { markClean: KeyChord; markProblem: KeyChord; skip: KeyChord };
  platform: Platform;
  /** Why proposed titles and artists could not be read; `null` when they were. */
  suggestionsUnavailable: string | null;
  /** The tag panel (wbx-65), between the pickers and the actions. */
  tags?: ReactNode;
}

export function IntakeTrackPanel(props: IntakeTrackPanelProps) {
  const { track, energy, prepColour, colours, coloursFailed, onRetryColours, busy, failure, note, keys, platform } = props;
  const t = useTranslation();
  const [pendingEnergy, setPendingEnergy] = useState<number | null>(null);
  // The Library's value arriving (or another track) ends the pending one.
  useEffect(() => { setPendingEnergy(null); }, [energy, track.id]);
  const shownEnergy = pendingEnergy ?? energy;
  const clean = colours?.clean ?? null;

  const setEnergy = (level: number) => {
    const to = level === shownEnergy ? 0 : level;
    setPendingEnergy(to);
    void props.onEnergy(to).then((written) => { if (!written) setPendingEnergy(null); });
  };

  const key = (chord: KeyChord) => (chord.key === "" ? null : <kbd className={styles.key}>{describeChord(chord, platform)}</kbd>);

  return (
    <section className={styles.panel} aria-label={t("Track")} data-testid="intake-panel">
      <div className={styles.fields}>
        <InPlaceField key={`title:${track.id}`} label={t("Title")} value={track.title} onCommit={props.onTitle} onSaveThenMark={props.onSaveThenMark} />
        <InPlaceField key={`artist:${track.id}`} label={t("Artist")} value={track.artist} onCommit={props.onArtist} onSaveThenMark={props.onSaveThenMark} />
      </div>
      {track.suggestions.length > 0 ? (
        <ul className={styles.suggestions} aria-label={t("Proposed")}>
          {track.suggestions.map((suggestion) => (
            <li key={`${suggestion.field}:${suggestion.value}`}>
              <span className={styles.label}>{t(suggestion.field)}</span> {suggestion.value}
              <span className={styles.source}> {suggestion.source} · {suggestion.confidence.toFixed(2)}</span>
            </li>
          ))}
        </ul>
      ) : null}

      <div className={styles.pickers}>
        <div className={styles.picker}>
          <span className={styles.label} id="intake-energy-label">{t("Energy")}</span>
          <div className={styles.energy} role="radiogroup" aria-labelledby="intake-energy-label">
            {[1, 2, 3, 4, 5].map((level) => (
              <button
                key={level}
                type="button"
                role="radio"
                aria-checked={shownEnergy === level}
                aria-label={`${t("Energy")} ${level}`}
                title={`${t("Energy")} ${level} (Shift+${level})`}
                data-level={level}
                className={styles.level}
                onClick={() => setEnergy(level)}
              >
                {level}
              </button>
            ))}
          </div>
        </div>
        <div className={styles.picker}>
          <span className={styles.label} id="intake-prep-label">{t("Prep colour")}</span>
          {colours ? (
            <div className={styles.prep} role="radiogroup" aria-labelledby="intake-prep-label">
              {colours.labels.map((colour) => {
                const on = prepColour?.id === colour.id;
                return (
                  <button
                    key={colour.id}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    aria-label={colour.label}
                    title={colour.label === colour.colour ? colour.colour : `${colour.label} (${colour.colour})`}
                    className={styles.chip}
                    disabled={busy}
                    onClick={() => props.onPrepColour(on ? null : colour)}
                  >
                    <span className={styles.swatch} data-prep={colour.colour} aria-hidden="true" />
                    {colour.label}
                  </button>
                );
              })}
            </div>
          ) : (
            <p className={styles.hint} data-testid="intake-colours-unavailable">
              {coloursFailed !== null ? (
                <>
                  {t("The Prep colours could not be read")}: {coloursFailed}{" "}
                  <button type="button" className={styles.small} onClick={onRetryColours}>{t("Retry")}</button>
                </>
              ) : t("Reading the Prep colours…")}
            </p>
          )}
        </div>
      </div>

      {props.tags}

      {props.suggestionsUnavailable !== null ? (
        <p className={styles.hint} data-testid="intake-suggestions-unavailable">{t("Proposed titles and artists could not be read")}: {props.suggestionsUnavailable}</p>
      ) : null}
      {failure !== null ? <p className={styles.failed} data-testid="intake-edit-failed" role="alert">{failure}</p> : null}
      {note !== null ? <p className={styles.note} data-testid="intake-note" role="status">{note}</p> : null}

      <div className={styles.actions} data-testid="intake-actions">
        <button
          type="button"
          className={styles.markClean}
          disabled={busy || clean === null}
          title={clean === null ? t("No colour label is named Clean, so there is no Clean to mark.") : `${clean.label} (${clean.colour})`}
          onClick={props.onMarkClean}
        >
          {t("Mark Clean and next")}
        </button>
        {key(keys.markClean)}
        <span className={styles.open} data-testid="intake-panel-needs">{t(needsLine(track))}</span>
        {clean === null
          ? <span className={styles.noClean} data-testid="intake-no-clean">{t("No colour label is named Clean, so there is no Clean to mark.")}</span>
          : null}
        <span className={styles.secondary}>
          {key(keys.markProblem)}
          <button type="button" className={styles.small} disabled={busy || colours?.problem == null} onClick={props.onMarkProblem}>
            {t("Mark Problem")}
          </button>
          {key(keys.skip)}
          <button type="button" className={styles.small} disabled={busy} onClick={props.onSkip}>{t("Skip")}</button>
        </span>
      </div>
    </section>
  );
}

/**
 * A field edited in place. What the DJ types is a draft; Enter or leaving
 * the field saves it, Escape drops it and shows the latest saved value.
 * The baseline a draft is compared with is the latest saved value: the
 * Library's (a value arriving from an agent or the CLI moves it) or the
 * DJ's own last save, so consecutive saves without leaving the field each
 * write. A draft the DJ has typed is never overwritten by a value from
 * elsewhere; with nothing typed, the field shows the new value.
 *
 * One save at a time: a second asks after the first. Ctrl+Enter saves and
 * then marks the track Clean (`onSaveThenMark`, which guards the whole
 * operation); a held key's repeats do nothing. A draft still unsaved when
 * the track changes (another row chosen) is saved for the track it was
 * typed for.
 */
function InPlaceField({ label, value, onCommit, onSaveThenMark }: {
  label: string;
  value: string;
  onCommit: (value: string) => Promise<boolean>;
  onSaveThenMark: (save: () => Promise<boolean>) => void;
}) {
  // The DJ's own text; `null` when they have typed nothing of their own.
  const [draft, setDraft] = useState<string | null>(null);
  // The latest saved value: what a draft is compared with, and what Escape shows.
  const [saved, setSaved] = useState(value);
  const [saving, setSaving] = useState(false);
  const latest = useRef({ draft, saved, commit: onCommit });
  latest.current = { ...latest.current, draft, commit: onCommit };
  const inFlight = useRef<Promise<boolean> | null>(null);

  // A new value from the Library moves the baseline.
  useEffect(() => {
    latest.current.saved = value;
    setSaved(value);
  }, [value]);

  const save = async (): Promise<boolean> => {
    // After any save already on its way.
    if (inFlight.current) await inFlight.current;
    const typed = latest.current.draft;
    if (typed === null || typed === latest.current.saved) {
      if (typed !== null) { latest.current.draft = null; setDraft(null); }
      return true;
    }
    const writing = (async () => {
      setSaving(true);
      const written = await latest.current.commit(typed);
      if (written) {
        latest.current.saved = typed;
        setSaved(typed);
      }
      // Unless the DJ has typed on since, the draft is done with (saved, or
      // refused and back to the saved value).
      if (latest.current.draft === typed) {
        latest.current.draft = null;
        setDraft(null);
      }
      setSaving(false);
      return written;
    })();
    inFlight.current = writing;
    try {
      return await writing;
    } finally {
      if (inFlight.current === writing) inFlight.current = null;
    }
  };

  // Unmounted with a change typed (another track chosen): save it for this one.
  useEffect(() => () => {
    const left = latest.current;
    if (left.draft !== null && left.draft !== left.saved) void left.commit(left.draft);
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // Every key typed here is the field's: never the deck's, the list's or the view's.
    event.stopPropagation();
    if (event.key === "Enter") {
      event.preventDefault();
      if (event.repeat) return;
      if (event.ctrlKey || event.metaKey) onSaveThenMark(save);
      else void save();
    } else if (event.key === "Escape") {
      event.preventDefault();
      latest.current.draft = null;
      setDraft(null);
    }
  };

  const id = `intake-field-${label}`;
  return (
    <label className={styles.field} htmlFor={id}>
      <span className={styles.label}>{label}</span>
      <input
        id={id}
        className={styles.input}
        type="text"
        spellCheck={false}
        value={draft ?? saved}
        data-pending={saving ? "" : undefined}
        onChange={(event) => {
          latest.current.draft = event.target.value;
          setDraft(event.target.value);
        }}
        onBlur={() => { void save(); }}
        onKeyDown={onKeyDown}
      />
    </label>
  );
}
