import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { FilterValues, KeyDisplay } from "@/ipc/types";
import { formatKey } from "@/lib/preferences";
import { OWN_ACTIVATION } from "@/lib/shortcuts";
import {
  EMPTY_SHELL_FILTER, ENERGY_LEVELS, clearCategory, isFiltering, keyChoicePicked, keyChoices, pickSummaries, setBpmRange,
  setEnergyRange, setTagsAny, toggleKeyChoice, togglePrepColour, toggleTag, type KeyChoice, type ShellFilter,
} from "@/lib/shellTrackFilter";
import { COLOR_NAMES } from "@/lib/trackFilter";
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
  values: FilterValues | null;
  valuesError: string | null;
  count: FilterCount;
  /** How the list writes and sorts keys (wbx-56): `view.keyDisplay`, `view.keySort`. */
  keyDisplay: KeyDisplay;
  keySort: "auto" | "alphabetical" | "musical";
}

/**
 * The Track Filter bar above the Library's list (wbx-57): one control per
 * MyTag category, then Energy, Prep colour, BPM and key, the text, the
 * "n of m" count and Clear. A control with something picked is outlined in
 * the accent and says what is picked; opening one shows its choices in a row
 * under the bar. Every control is a real button, select or field, so the bar
 * works from the keyboard, and the bar owns Space and Enter so they never
 * reach the deck.
 */
export function TrackFilterBar({ model, onChange, values, valuesError, count, keyDisplay, keySort }: TrackFilterBarProps) {
  const [open, setOpen] = useState<Panel | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const keyName = (stored: string) => formatKey(stored, keyDisplay);
  const keys = keyChoices(values?.keys ?? [], keyDisplay, keySort);
  const said = pickSummaries(model, keyName);
  const categories = values?.tags ?? [];
  const filtering = isFiltering(model);

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

  const control = (panel: Panel, name: string, picked: string | null) => {
    const id = panelId(panel);
    const expanded = open !== null && panelId(open) === id;
    return (
      <button
        key={id}
        type="button"
        ref={(el) => { if (el) buttons.current.set(id, el); else buttons.current.delete(id); }}
        className={styles.control}
        data-active={picked !== null ? "" : undefined}
        aria-expanded={expanded}
        aria-controls={expanded ? "track-filter-panel" : undefined}
        onClick={() => setOpen(expanded ? null : panel)}
      >
        <span className={styles.controlName}>{name}</span>
        {picked !== null ? <> <span className={styles.picked}>{picked}</span></> : <span className={styles.caret} aria-hidden> ▾</span>}
      </button>
    );
  };

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
          <input
            className={styles.text}
            type="search"
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
          disabled={!filtering}
          onClick={() => {
            onChange(EMPTY_SHELL_FILTER);
            setOpen(null);
          }}
        >
          Clear filter
        </button>
      </div>
      {valuesError !== null ? <p className={styles.problem} role="alert">The filter's choices could not be read: {valuesError}</p> : null}
      {open !== null ? (
        <div id="track-filter-panel" className={styles.panel}>
          <PanelBody panel={open} model={model} onChange={onChange} values={values} keys={keys} />
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

function PanelBody({ panel, model, onChange, values, keys }: {
  panel: Panel;
  model: ShellFilter;
  onChange: (next: ShellFilter) => void;
  values: FilterValues | null;
  keys: KeyChoice[];
}) {
  switch (panel.kind) {
    case "tags": {
      const category = values?.tags.find((c) => c.name === panel.category);
      const pick = model.myTags.find((p) => p.category === panel.category);
      return (
        <>
          <div role="group" aria-label={`${panel.category} tags`} className={styles.choices}>
            {(category?.tags ?? []).map((tag) => (
              <Toggle key={tag} name={tag} pressed={pick?.tags.includes(tag) ?? false}
                onPress={() => onChange(toggleTag(model, panel.category, tag))} />
            ))}
            {category && category.tags.length === 0 ? <span className={styles.empty}>No tags in {panel.category}</span> : null}
          </div>
          <div role="group" aria-label="Match" className={styles.choices}>
            <Toggle name="All" pressed={!(pick?.any ?? false)} onPress={() => onChange(setTagsAny(model, panel.category, false))} />
            <Toggle name="Any" pressed={pick?.any ?? false} onPress={() => onChange(setTagsAny(model, panel.category, true))} />
            <button type="button" className={styles.choice} disabled={!pick} onClick={() => onChange(clearCategory(model, panel.category))}>
              Clear
            </button>
          </div>
        </>
      );
    }
    case "energy": {
      const from = model.energy?.min ?? 1;
      const to = model.energy?.max ?? 5;
      const levels = ENERGY_LEVELS.map((level) => <option key={level} value={level}>{level}</option>);
      return (
        <div className={styles.choices}>
          <label className={styles.field}>From
            <select aria-label="Energy from" value={from} onChange={(e) => onChange(setEnergyRange(model, Number(e.target.value), to))}>{levels}</select>
          </label>
          <label className={styles.field}>To
            <select aria-label="Energy to" value={to} onChange={(e) => onChange(setEnergyRange(model, from, Number(e.target.value)))}>{levels}</select>
          </label>
        </div>
      );
    }
    case "prep":
      return (
        <div role="group" aria-label="Prep colours" className={styles.choices}>
          {COLOR_NAMES.map((colour) => (
            <Toggle key={colour} name={colour} swatch={colour} pressed={model.prepColours.includes(colour)}
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
            <Toggle key={choice.label} name={choice.label} pressed={keyChoicePicked(model, choice)}
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
