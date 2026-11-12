/**
 * The Intake tag panel (wbx-65, `docs/design/screens/intake`): every
 * dimension of the DJ's tags as chips clicked to toggle, drawn from the
 * description the Library service reports. The dimensions that reach the
 * deck as MyTag categories carry a diamond and come first. Nothing here
 * knows how many dimensions there are or what they are called.
 *
 * State and writes are the Intake view's `TagStore` (`src/lib/tagStore.ts`):
 * a click shows at once and is written through `intake.set_tags`, one step
 * of the shared Undo. The panel reads again on every Library change, every
 * Sidecar change, when the Library is ready, and every few seconds while
 * visible (so a Tag schema edited on disk shows without a click).
 *
 * A chip toggles by pointer, Space or Enter; a held key toggles once.
 * Every key on a chip is the chip's (`data-own-activation`, and stopped
 * here), so it never reaches the deck. In the genre tree a child implies
 * its parents, deck-facing or not; turning a parent off narrows to it. A
 * single-tag dimension replaces its tag. Values no tag stands for are
 * dashed chips marked "not in the schema"; clicking one takes it off.
 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import { getBackend } from "@/ipc/client";
import { tagState, type PanelDimension } from "@/lib/tagPanel";
import type { TagStore } from "@/lib/tagStore";
import styles from "./IntakeTagPanel.module.css";

/** How often the panel reads again while visible: a schema edited on disk shows this soon. */
export const TAG_PANEL_POLL_MS = 3000;

function useStore(store: TagStore): number {
  return useSyncExternalStore(store.subscribe, store.getRevision);
}

/** Keeps `trackId`'s panel read: on mount, on Library and Sidecar changes, and while visible. */
function useTagReads(store: TagStore, trackId: string) {
  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    store.refresh(trackId);
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stops.push(backend.onLibraryChanged(() => store.refresh(trackId)));
      stops.push(backend.onLibraryReady(() => store.refresh(trackId)));
      stops.push(backend.onSidecarChanged((change) => { if (change.trackId === trackId || change.trackId === "") store.refresh(trackId); }));
    })();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") store.refresh(trackId);
    }, TAG_PANEL_POLL_MS);
    return () => {
      live = false;
      window.clearInterval(timer);
      for (const stop of stops) stop();
    };
  }, [store, trackId]);
}

export interface IntakeTagPanelProps {
  store: TagStore;
  trackId: string;
}

export function IntakeTagPanel({ store, trackId }: IntakeTagPanelProps) {
  const t = useTranslation();
  useStore(store);
  useTagReads(store, trackId);
  const { panel, failed } = store.panel(trackId);
  const anyDeck = panel?.dimensions.some((d) => d.deck) ?? false;
  const factsUnavailable = panel?.track?.factsUnavailable ?? null;
  const failures = store.failures().filter((failure) => failure.track === trackId);

  return (
    <section className={styles.tags} aria-label={t("Tags")} data-testid="intake-tags" data-track={trackId} data-own-activation>
      {failed !== null ? (
        <p className={styles.hint} data-testid="intake-tags-unavailable">
          {panel ? t("The tags could not be refreshed") : t("The tags could not be read")}: {failed}{" "}
          <button type="button" className={styles.small} onClick={() => store.refresh(trackId)}>{t("Retry")}</button>
        </p>
      ) : panel === null ? <p className={styles.hint}>{t("Reading the tags…")}</p> : null}
      {panel?.schemaProblem ? <p className={styles.hint} data-testid="intake-tags-schema-problem">{panel.schemaProblem}</p> : null}
      {factsUnavailable !== null ? <p className={styles.hint}>{t("Tags kept as Facts could not be read")}: {factsUnavailable}</p> : null}
      {failures.length > 0 ? (
        <ul className={styles.failures} data-testid="intake-tags-failed" role="alert">
          {failures.map((failure) => (
            <li key={failure.key} data-dimension={failure.dimension} data-kind={failure.kind}>
              {failure.kind === "failed" ? t("Not saved") : t("Saved, but not recorded as your Fact")}: {failure.label}: {failure.message}{" "}
              {failure.kind === "failed" ? <button type="button" className={styles.small} onClick={() => store.retry(failure.key)}>{t("Retry")}</button> : null}
              <button type="button" className={styles.small} onClick={() => store.acknowledge(failure.key)}>{t("Dismiss")}</button>
            </li>
          ))}
        </ul>
      ) : null}
      {anyDeck ? <p className={styles.legend}><span className={styles.diamond} aria-hidden="true">◆</span> {t("Shows in the Track Filter on the deck")}</p> : null}
      {panel && panel.dimensions.length === 0 ? <p className={styles.hint}>{t("There are no tags to set: the Library has no MyTags and there is no Tag schema.")}</p> : null}
      <div className={styles.dimensions}>
        {panel?.dimensions.map((dimension, at) => (
          <DimensionChips key={dimension.id} store={store} trackId={trackId} dimension={dimension} hue={at % 6} disabled={dimension.edit === "fact" && factsUnavailable !== null} />
        ))}
      </div>
    </section>
  );
}

function DimensionChips({ store, trackId, dimension, hue, disabled }: { store: TagStore; trackId: string; dimension: PanelDimension; hue: number; disabled: boolean }) {
  const t = useTranslation();
  const set = store.shown(trackId, dimension);
  const unmapped = store.unmapped(trackId, dimension);
  const own = (act: () => void) => (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    // The chip's own: never the deck's or the view's, and once per press.
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) act();
  };
  return (
    <div className={styles.dimension} role="group" aria-label={dimension.label} data-deck={dimension.deck ? "true" : "false"} data-single={dimension.single ? "true" : undefined} data-hue={hue}>
      <span className={styles.label} aria-hidden="true">
        {dimension.label}
        {dimension.deck ? <span className={styles.diamond} title={t("Reaches the deck as a MyTag category")}> ◆</span> : null}
        {dimension.single ? <span className={styles.single}> · {t("one")}</span> : null}
      </span>
      <div className={styles.chips}>
        {dimension.tags.map((tag) => {
          const state = tagState(dimension, set, tag.id);
          const notOnDeck = dimension.edit === "my_tag" && tag.myTag === null;
          return (
            <button
              key={tag.id}
              type="button"
              className={styles.chip}
              data-tag={tag.id}
              data-my-tag={tag.myTag ?? undefined}
              data-state={state}
              data-depth={tag.depth}
              aria-pressed={state === "set"}
              disabled={notOnDeck || disabled}
              title={notOnDeck
                ? t("Not on the deck yet: the Library has no MyTag for this tag until the Tag schema is projected.")
                : state === "implied" ? t("Implied by a more specific tag; click to narrow to this one") : undefined}
              onClick={() => store.toggle(trackId, dimension, tag.id)}
              onKeyDown={own(() => store.toggle(trackId, dimension, tag.id))}
            >
              {tag.name}
            </button>
          );
        })}
        {unmapped.map((value) => (
          <button
            key={`unmapped:${value.id}`}
            type="button"
            className={styles.chip}
            data-unmapped={value.id}
            data-state="unmapped"
            aria-label={`${value.name} (${t("not in the Tag schema")}; ${t("click to take it off")})`}
            title={t("Not in the Tag schema: kept until you take it off. Click to take it off.")}
            onClick={() => store.dropUnmapped(trackId, dimension, value.id)}
            onKeyDown={own(() => store.dropUnmapped(trackId, dimension, value.id))}
          >
            {value.name}
          </button>
        ))}
      </div>
    </div>
  );
}

/** Failures of tag writes on tracks other than `shownTrack`: kept until acknowledged or retried. */
export function TagFailures({ store, titleOf, shownTrack }: { store: TagStore; titleOf: (id: string) => string; shownTrack: string | null }) {
  const t = useTranslation();
  useStore(store);
  const failures = store.failures().filter((failure) => failure.track !== shownTrack);
  if (failures.length === 0) return null;
  return (
    <ul className={styles.failures} data-testid="intake-tag-failures" role="alert" data-own-activation="">
      {failures.map((failure) => (
        <li key={failure.key} data-track={failure.track}>
          {failure.kind === "failed" ? t("Tags not saved") : t("Tags saved, not recorded as your Fact")} — “{titleOf(failure.track)}”, {failure.label}: {failure.message}{" "}
          {failure.kind === "failed" ? <button type="button" className={styles.small} onClick={() => store.retry(failure.key)}>{t("Retry")}</button> : null}
          <button type="button" className={styles.small} onClick={() => store.acknowledge(failure.key)}>{t("Dismiss")}</button>
        </li>
      ))}
    </ul>
  );
}

/**
 * The palette's way to tag (wbx-65): a dialog listing every tag as
 * "Dimension: Tag", filtered as the DJ types; Up and Down move, Enter
 * toggles the chosen tag and closes, Escape closes. Focus goes back to
 * where it was when the picker opened.
 */
export function TagPicker({ store, trackId, onClose }: { store: TagStore; trackId: string; onClose: () => void }) {
  const t = useTranslation();
  useStore(store);
  useTagReads(store, trackId);
  const [query, setQuery] = useState("");
  const [at, setAt] = useState(0);
  const returnTo = useRef<Element | null>(typeof document === "undefined" ? null : document.activeElement);
  const panel = store.panel(trackId).panel;
  const options = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return (panel?.dimensions ?? []).flatMap((dimension) => dimension.tags
      .filter((tag) => dimension.edit !== "my_tag" || tag.myTag !== null)
      .map((tag) => ({ dimension, tag, text: `${dimension.label}: ${tag.name}` })))
      .filter((option) => words.every((word) => option.text.toLowerCase().includes(word)));
  }, [panel, query]);
  const close = () => {
    onClose();
    const back = returnTo.current;
    if (back instanceof HTMLElement && back.isConnected) back.focus();
  };
  const choose = (index: number) => {
    const option = options[index];
    if (!option) return;
    store.toggle(trackId, option.dimension, option.tag.id);
    close();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === "Escape") { event.preventDefault(); close(); }
    else if (event.key === "ArrowDown") { event.preventDefault(); setAt((i) => Math.min(i + 1, Math.max(options.length - 1, 0))); }
    else if (event.key === "ArrowUp") { event.preventDefault(); setAt((i) => Math.max(i - 1, 0)); }
    else if (event.key === "Enter") { event.preventDefault(); if (!event.repeat) choose(at); }
  };
  return (
    <div className={styles.backdrop} onPointerDown={(event) => { if (event.target === event.currentTarget) close(); }}>
      <div className={styles.picker} role="dialog" aria-modal="true" aria-label={t("Tag the track")} data-testid="tag-picker" onKeyDown={onKeyDown} data-own-activation="">
        <input
          className={styles.search}
          aria-label={t("Find a tag")}
          placeholder={t("Find a tag: dimension or tag")}
          autoFocus
          value={query}
          onChange={(event) => { setQuery(event.target.value); setAt(0); }}
        />
        <ul className={styles.options} role="listbox" aria-label={t("Tags")}>
          {options.map((option, index) => {
            const state = tagState(option.dimension, store.shown(trackId, option.dimension), option.tag.id);
            return (
              <li
                key={`${option.dimension.id}:${option.tag.id}`}
                role="option"
                aria-selected={index === at}
                data-state={state}
                className={styles.option}
                onPointerDown={(event) => { event.preventDefault(); choose(index); }}
              >
                {option.text}{state === "set" ? ` ✓` : state === "implied" ? ` (${t("implied")})` : ""}
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
