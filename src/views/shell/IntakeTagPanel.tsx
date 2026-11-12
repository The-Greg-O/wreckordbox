/**
 * The Intake tag panel (wbx-65, `docs/design/screens/intake`): every
 * dimension of the DJ's tags as chips clicked to toggle, drawn from the
 * description the Library service reports. The dimensions that reach the
 * deck as MyTag categories carry a diamond and come first. Nothing here
 * knows how many dimensions there are or what they are called.
 *
 * A chip toggles by pointer, Space or Enter; a held key toggles once.
 * Every key on a chip is the chip's (`data-own-activation`, and stopped
 * here), so it never reaches the deck. In the genre tree a child implies
 * its parents (shown as implied); turning a parent off takes off what
 * implies it. A deck-facing tag with no MyTag in the Library yet (a Tag
 * schema not projected) cannot be set, and says why.
 */
import type { KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import { tagState } from "@/lib/tagPanel";
import styles from "./IntakeTagPanel.module.css";
import { useTagPanel, type TagWriters } from "./useTagPanel";

export interface IntakeTagPanelProps extends TagWriters {
  trackId: string;
}

export function IntakeTagPanel({ trackId, putMyTag, setFact }: IntakeTagPanelProps) {
  const t = useTranslation();
  const { panel, failed, failure, shown, toggle, retry } = useTagPanel(trackId, { putMyTag, setFact });
  const anyDeck = panel?.dimensions.some((d) => d.deck) ?? false;
  const factsUnavailable = panel?.track?.factsUnavailable ?? null;

  return (
    <section className={styles.tags} aria-label={t("Tags")} data-testid="intake-tags" data-track={trackId} data-own-activation>
      {failed !== null ? (
        <p className={styles.hint} data-testid="intake-tags-unavailable">
          {panel ? t("The tags could not be refreshed") : t("The tags could not be read")}: {failed}{" "}
          <button type="button" className={styles.small} onClick={retry}>{t("Retry")}</button>
        </p>
      ) : panel === null ? <p className={styles.hint}>{t("Reading the tags…")}</p> : null}
      {panel?.schemaProblem ? <p className={styles.hint} data-testid="intake-tags-schema-problem">{panel.schemaProblem}</p> : null}
      {factsUnavailable !== null ? <p className={styles.hint}>{t("Tags kept as Facts could not be read")}: {factsUnavailable}</p> : null}
      {failure !== null ? <p className={styles.failed} data-testid="intake-tags-failed" role="alert">{t("Not saved")}: {failure}</p> : null}
      {anyDeck ? <p className={styles.legend}><span className={styles.diamond} aria-hidden="true">◆</span> {t("Shows in the Track Filter on the deck")}</p> : null}
      {panel && panel.dimensions.length === 0 ? <p className={styles.hint}>{t("There are no tags to set: the Library has no MyTags and there is no Tag schema.")}</p> : null}
      <div className={styles.dimensions}>
        {panel?.dimensions.map((dimension, at) => {
          const set = shown(dimension);
          const unknown = dimension.edit === "fact" && factsUnavailable !== null;
          return (
            <div key={dimension.id} className={styles.dimension} role="group" aria-label={dimension.label} data-deck={dimension.deck ? "true" : "false"} data-hue={at % 6}>
              <span className={styles.label} aria-hidden="true">
                {dimension.label}
                {dimension.deck ? <span className={styles.diamond} title={t("Reaches the deck as a MyTag category")}> ◆</span> : null}
              </span>
              <div className={styles.chips}>
                {dimension.tags.map((tag) => {
                  const state = tagState(dimension, set, tag.id);
                  const notOnDeck = dimension.edit === "my_tag" && tag.myTag === null;
                  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
                    if (event.key !== "Enter" && event.key !== " ") return;
                    // The chip's own: never the deck's or the view's, and once per press.
                    event.preventDefault();
                    event.stopPropagation();
                    if (!event.repeat) toggle(dimension, tag.id);
                  };
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
                      disabled={notOnDeck || unknown}
                      title={notOnDeck
                        ? t("Not on the deck yet: the Library has no MyTag for this tag until the Tag schema is projected.")
                        : state === "implied" ? t("Implied by a more specific tag") : undefined}
                      onClick={() => toggle(dimension, tag.id)}
                      onKeyDown={onKeyDown}
                    >
                      {tag.name}
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
