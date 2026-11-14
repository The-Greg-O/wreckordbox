import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { FilterValues, KeyDisplay, TagCategory } from "@/ipc/types";
import { formatKey } from "@/lib/preferences";
import { OWN_ACTIVATION } from "@/lib/shortcuts";
import {
  BPM_MATCH_PCT, EMPTY_SHELL_FILTER, ENERGY_LEVELS, PREP_CHOICES, bpmBand, clearCategory, dropStale, hasPicks,
  keyChoicePicked, keyChoices, matchedKeys, pickSummaries, setBpmRange, setEnergyRange, setTagsAny, tagsAny,
  toggleKeyChoice, togglePrepColour, toggleTag, type KeyChoice, type LoadedTrack, type ShellFilter,
} from "@/lib/shellTrackFilter";
import type { FilterCount } from "./useShellTrackFilter";
import styles from "./TrackFilterBar.module.css";

/** Which control's panel is open: a MyTag category by name, or a field. */
type Panel = { kind: "tags"; category: string } | { kind: "energy" | "prep" | "bpm" | "key" };

function panelId(panel: Panel): string {
  return panel.kind === "tags" ? `tags:${panel.category}` : panel.kind;
}

export interface TrackFilterBarProps {
  model: ShellFilter;
  onChange: (next: ShellFilter) => void;
  /** The open list's choices (its keys). */
  values: FilterValues | null;
  valuesError: string | null;
  /** The MyTag categories to offer: the Library's, or on a Stick list the Stick's own. */
  categories: readonly TagCategory[];
  /** Picks the Library no longer holds, just left out of the filter. */
  dropped: readonly string[];
  /** Selected tracks the filter hid, just deselected; `0` for none. */
  deselected: number;
  /** A Stick's list: picks the Stick does not hold are left out there, and said (wbx-45.12). */
  onStick: boolean;
  count: FilterCount;
  /** Whether anything narrows the open list now (on a Stick, only the picks it holds). */
  filtering: boolean;
  /** The track Key match and BPM ±6% follow. */
  loaded: LoadedTrack;
  /** How the list writes and sorts keys (wbx-56): `view.keyDisplay`, `view.keySort`. */
  keyDisplay: KeyDisplay;
  keySort: "auto" | "alphabetical" | "musical";
  /** "Save as intelligent playlist" (wbx-58): its button for the row, and what goes under it. */
  save?: { control: ReactNode; panel: ReactNode };
}

/**
 * The Track Filter bar above the Library's list (wbx-57): one control per
 * MyTag category, then Energy, Prep colour, BPM and key, Key match and
 * BPM ±6% against the loaded track, the text, the "n of m" count and Clear.
 * A control with something picked is outlined in the accent and says what is
 * picked; opening one shows its choices in a row under the bar. Every control
 * is a real button, select or field, so the bar works from the keyboard, and
 * the bar owns Space and Enter so they never reach the deck.
 */
export function TrackFilterBar(props: TrackFilterBarProps) {
  const { model, onChange, values, valuesError, categories, dropped, deselected, onStick, count, filtering, loaded, keyDisplay, keySort, save } = props;
  const [open, setOpen] = useState<Panel | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const keyName = (identity: string) => formatKey(identity, keyDisplay);
  const said = pickSummaries(model, loaded, keyName);
  const keys = keyChoices(values?.keys ?? [], model.keys, keyDisplay, keySort);
  const matched = matchedKeys(loaded);
  const band = bpmBand(loaded);

  // A press anywhere outside the bar closes its panel.
  useEffect(() => {
    if (open === null) return;
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(null);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open]);

  const close = () => {
    if (open === null) return;
    const button = buttons.current.get(panelId(open));
    setOpen(null);
    button?.focus();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape" && open !== null) {
      event.preventDefault();
      event.stopPropagation();
      close();
    }
  };

  const control = (panel: Panel, name: string, picked: string | null, unavailable?: string) => {
    const id = panelId(panel);
    const expanded = open !== null && panelId(open) === id;
    return (
      <button
        key={id}
        type="button"
        ref={(el) => { if (el) buttons.current.set(id, el); else buttons.current.delete(id); }}
        className={styles.control}
        data-active={picked !== null && unavailable === undefined ? "" : undefined}
        aria-expanded={expanded}
        aria-controls={expanded ? "track-filter-panel" : undefined}
        aria-disabled={unavailable !== undefined || undefined}
        title={unavailable}
        onClick={() => { if (unavailable === undefined) setOpen(expanded ? null : panel); }}
      >
        <span className={styles.controlName}>{name}</span>
        {picked !== null ? <> <span className={styles.picked}>{picked}</span></> : <span className={styles.caret} aria-hidden> ▾</span>}
      </button>
    );
  };

  /** Key match / BPM ±6%: on or off; lit only when it filters. */
  const follow = (name: string, on: boolean, live: string | null, flip: () => void, reason: string) => (
    <button
      type="button"
      className={styles.control}
      aria-pressed={on}
      data-active={on && live !== null ? "" : undefined}
      title={live === null ? reason : undefined}
      onClick={flip}
    >
      <span className={styles.controlName}>{name}</span>
      {on ? <> <span className={styles.picked}>{live ?? "no track loaded"}</span></> : null}
    </button>
  );

  // On a Stick list, once its own categories are read: the picks it does not hold.
  const notOnStick = onStick && values !== null && valuesError === null ? dropStale(model, categories).dropped : [];

  return (
    <div ref={root} className={styles.bar} role="search" aria-label="Track Filter" {...{ [OWN_ACTIVATION]: "" }} onKeyDown={onKeyDown}>
      <div className={styles.row}>
        <span className={styles.label} aria-hidden>Track Filter</span>
        <div className={styles.controls}>
          {categories.map((c) => control({ kind: "tags", category: c.name }, c.name, said.myTags[c.name] ?? null))}
          {control({ kind: "energy" }, "Energy", said.energy)}
          {control({ kind: "prep" }, "Prep colour", said.prepColours)}
          {control({ kind: "bpm" }, "BPM", said.bpm)}
          {control({ kind: "key" }, "Key", said.keys)}
          {follow("Key match", model.keyMatch, matched === null ? null : formatKey(loaded.key ?? "", keyDisplay),
            () => onChange({ ...model, keyMatch: !model.keyMatch }),
            loaded.key === null ? "Follows the loaded track's key: no track is loaded" : "The loaded track's key is not on the wheel")}
          {follow(`BPM ±${BPM_MATCH_PCT}%`, model.bpmMatch, band === null ? null : ((loaded.bpmX100 ?? 0) / 100).toFixed(2),
            () => onChange({ ...model, bpmMatch: !model.bpmMatch }),
            "Follows the loaded track's tempo: no track is loaded")}
          <input
            className={styles.text}
            type="text"
            aria-label="Filter text"
            placeholder="Text"
            value={model.text}
            onChange={(event) => onChange({ ...model, text: event.target.value })}
          />
        </div>
        <span className={styles.count} role="status" data-state={count.state}>{countText(count, filtering)}</span>
        <button
          type="button"
          className={styles.clear}
          disabled={!hasPicks(model) && model.text.trim() === ""}
          onClick={() => {
            onChange(EMPTY_SHELL_FILTER);
            setOpen(null);
          }}
        >
          Clear filter
        </button>
        {save?.control}
      </div>
      {save?.panel}
      {notOnStick.length > 0 ? <p className={styles.note} role="note">Not on this Stick, left out here: {notOnStick.join(", ")}</p> : null}
      {dropped.length > 0 ? <p className={styles.note} role="note">Left out of the filter, no longer in the Library: {dropped.join(", ")}</p> : null}
      {deselected > 0 ? (
        <p className={styles.note} role="note">
          {deselected === 1 ? "1 selected track the filter hides was deselected" : `${deselected} selected tracks the filter hides were deselected`}
        </p>
      ) : null}
      {valuesError !== null ? <p className={styles.problem} role="alert">The filter's choices could not be read: {valuesError}</p> : null}
      {open !== null ? (
        <div id="track-filter-panel" className={styles.panel}>
          <PanelBody panel={open} model={model} onChange={onChange} categories={categories} keys={keys} values={values} />
        </div>
      ) : null}
    </div>
  );
}

function countText(count: FilterCount, filtering: boolean): string {
  switch (count.state) {
    case "reading":
      return "Counting…";
    case "failed":
      return `Count unavailable: ${count.message}`;
    case "known":
      return filtering ? `${count.matching} of ${count.total} tracks` : `${count.total} tracks`;
  }
}

function Toggle({ name, pressed, onPress, swatch }: { name: string; pressed: boolean; onPress: () => void; swatch?: string }) {
  return (
    <button type="button" className={styles.choice} aria-pressed={pressed} onClick={onPress}>
      {swatch ? <span className={styles.swatch} data-colour={swatch} aria-hidden /> : null}
      {name}
    </button>
  );
}

function PanelBody({ panel, model, onChange, categories, keys, values }: {
  panel: Panel;
  model: ShellFilter;
  onChange: (next: ShellFilter) => void;
  categories: readonly TagCategory[];
  keys: KeyChoice[];
  values: FilterValues | null;
}) {
  switch (panel.kind) {
    case "tags": {
      const category = categories.find((c) => c.name === panel.category);
      const pick = model.myTags.find((p) => p.category === panel.category);
      const any = tagsAny(model, panel.category);
      return (
        <>
          <div role="group" aria-label={`${panel.category} tags`} className={styles.choices}>
            {(category?.tags ?? []).map((tag) => (
              <Toggle key={tag} name={tag} pressed={pick?.tags.includes(tag) ?? false}
                onPress={() => onChange(toggleTag(model, panel.category, tag))} />
            ))}
            {category && category.tags.length === 0 ? <span className={styles.empty}>No tags in {panel.category}</span> : null}
          </div>
          <div role="group" aria-label={`${panel.category} match`} className={styles.choices}>
            <Toggle name="All" pressed={!any} onPress={() => onChange(setTagsAny(model, panel.category, false))} />
            <Toggle name="Any" pressed={any} onPress={() => onChange(setTagsAny(model, panel.category, true))} />
            <button type="button" className={styles.choice} disabled={!pick} onClick={() => onChange(clearCategory(model, panel.category))}>
              Clear
            </button>
          </div>
        </>
      );
    }
    case "energy": {
      const levels = ENERGY_LEVELS.map((level) => <option key={level} value={level}>{level}</option>);
      const from = model.energy?.min;
      const to = model.energy?.max;
      const pick = (a: string, b: string) => onChange(setEnergyRange(model, a === "" ? null : Number(a), b === "" ? null : Number(b)));
      return (
        <div className={styles.choices}>
          <label className={styles.field}>From
            <select aria-label="Energy from" value={from ?? ""} onChange={(e) => pick(e.target.value, e.target.value === "" ? "" : String(to ?? 5))}>
              <option value="">Any</option>{levels}
            </select>
          </label>
          <label className={styles.field}>To
            <select aria-label="Energy to" value={to ?? ""} onChange={(e) => pick(e.target.value === "" ? "" : String(from ?? 1), e.target.value)}>
              <option value="">Any</option>{levels}
            </select>
          </label>
          <Toggle name="Unrated" pressed={model.unrated} onPress={() => onChange({ ...model, unrated: !model.unrated })} />
        </div>
      );
    }
    case "prep":
      return (
        <div role="group" aria-label="Prep colours" className={styles.choices}>
          {PREP_CHOICES.map((colour) => (
            <Toggle key={colour} name={colour === "none" ? "None" : colour} swatch={colour} pressed={model.prepColours.includes(colour)}
              onPress={() => onChange(togglePrepColour(model, colour))} />
          ))}
        </div>
      );
    case "bpm":
      return <BpmFields model={model} onChange={onChange} />;
    case "key":
      return (
        <div role="group" aria-label="Keys" className={styles.choices}>
          {keys.map((choice) => (
            <Toggle key={choice.identity} name={choice.label} pressed={keyChoicePicked(model, choice)}
              onPress={() => onChange(toggleKeyChoice(model, choice))} />
          ))}
          {values && keys.length === 0 ? <span className={styles.empty}>No keys in this list</span> : null}
        </div>
      );
  }
}

/** Two whole-BPM fields, applied on Enter or when focus leaves the panel's fields. */
function BpmFields({ model, onChange }: { model: ShellFilter; onChange: (next: ShellFilter) => void }) {
  const [from, setFrom] = useState(model.bpm?.min?.toString() ?? "");
  const [to, setTo] = useState(model.bpm?.max?.toString() ?? "");
  const parse = (text: string) => (text.trim() === "" ? null : Number(text));
  const apply = () => onChange(setBpmRange(model, parse(from), parse(to)));
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Enter") {
      event.preventDefault();
      apply();
    }
  };
  return (
    <div className={styles.choices}>
      <label className={styles.field}>From
        <input type="number" min={1} max={999} aria-label="BPM from" value={from}
          onChange={(e) => setFrom(e.target.value)} onKeyDown={onKey} onBlur={apply} />
      </label>
      <label className={styles.field}>To
        <input type="number" min={1} max={999} aria-label="BPM to" value={to}
          onChange={(e) => setTo(e.target.value)} onKeyDown={onKey} onBlur={apply} />
      </label>
    </div>
  );
}
