/**
 * The waveform theme chooser in the view shell's top bar (wbx-45.9;
 * docs/design/screens/intake/README.md: 3Band, RGB, Blue, Theme "chosen
 * from the top bar"; the mock draws it "wave 3Band ▾").
 *
 * It shows the theme every waveform draws (`useWaveformTheme`: the theme
 * file's `waveform`, else `view.waveformColor`) and writes the preference,
 * as Preferences > View > Waveform color and the deck menu do. While the
 * theme file sets it the button is greyed and says why; its menu still
 * opens, to show the reason, with every entry disabled.
 *
 * The menu is drawn inside the top bar, not portalled, so it takes the
 * shell's theme roles (a light Omarchy theme recolours it). The control owns
 * its activation (`data-own-activation`): Space and Enter open it and never
 * reach the deck; Up, Down, Home and End move; Escape or a click elsewhere
 * closes it and gives the focus back.
 */
import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import type { WaveformTheme } from "@/lib/preferences";
import { useWaveformTheme } from "@/lib/waveTheme";
import { usePreferencesContext, useTooltip } from "@/store/usePreferences";
import styles from "./WaveformThemeChooser.module.css";

/** In the design note's order; the labels Preferences uses. */
const CHOICES: readonly { value: WaveformTheme; label: string }[] = [
  { value: "3band", label: "3Band" },
  { value: "rgb", label: "RGB" },
  { value: "blue", label: "BLUE" },
  { value: "theme", label: "Theme" },
];

const REASON = "Set by the theme file (waveform).";

export function WaveformThemeChooser() {
  const t = useTranslation();
  const tooltip = useTooltip();
  const { update } = usePreferencesContext();
  const { palette, fromFile } = useWaveformTheme();
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const reasonId = useId();
  const label = CHOICES.find((choice) => choice.value === palette)?.label ?? palette;

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  };

  // A click anywhere else, or the window changing size, puts it away.
  useEffect(() => {
    if (!open) return;
    const outside = (event: MouseEvent) => {
      if (!box.current?.contains(event.target as Node)) setOpen(false);
    };
    const resize = () => setOpen(false);
    window.addEventListener("mousedown", outside, true);
    window.addEventListener("resize", resize);
    return () => {
      window.removeEventListener("mousedown", outside, true);
      window.removeEventListener("resize", resize);
    };
  }, [open]);

  // Opened, the entry in force takes the focus (the menu itself when the file holds them all).
  useEffect(() => {
    if (!open) return;
    const items = menu.current?.querySelectorAll<HTMLButtonElement>("[role='menuitemradio']:not(:disabled)") ?? [];
    const checked = [...items].find((item) => item.getAttribute("aria-checked") === "true");
    (checked ?? items[0] ?? menu.current)?.focus();
  }, [open]);

  const onKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!open) return;
    const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>("[role='menuitemradio']:not(:disabled)") ?? [])];
    const at = items.findIndex((item) => item === document.activeElement);
    const move = (to: number) => items[(to + items.length) % items.length]?.focus();
    switch (event.key) {
      case "ArrowDown": move(at + 1); break;
      case "ArrowUp": move(at < 0 ? items.length - 1 : at - 1); break;
      case "Home": move(0); break;
      case "End": move(items.length - 1); break;
      case "Escape": close(true); break;
      case "Tab": setOpen(false); return;
      default: return;
    }
    event.preventDefault();
    event.stopPropagation();
  };

  const choose = (value: WaveformTheme) => {
    if (fromFile) return;
    update("view", { waveformColor: value });
    close(true);
  };

  return (
    <div ref={box} className={styles.box} data-own-activation="" onKeyDown={onKey}>
      <button
        ref={trigger}
        type="button"
        className={styles.trigger}
        data-shell-wave-chooser=""
        aria-label={`${t("Waveform color")}: ${label}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-disabled={fromFile ? true : undefined}
        aria-describedby={fromFile ? reasonId : undefined}
        title={tooltip(fromFile ? t(REASON) : t("Waveform color"))}
        onClick={() => setOpen((was) => !was)}
      >
        <svg className={styles.glyph} viewBox="0 0 12 10" aria-hidden>
          <path d="M1 4v2M3 2v6M5 3v4M7 0v10M9 2v6M11 4v2" stroke="currentColor" strokeWidth="1.2" fill="none" />
        </svg>
        <span className={styles.word} aria-hidden>{t("wave")}</span>
        <span className={styles.name} data-wave-name="">{label}</span>
        <span className={styles.caret} aria-hidden>▾</span>
      </button>
      {fromFile ? <span id={reasonId} hidden>{t(REASON)}</span> : null}
      {open ? (
        <div ref={menu} className={styles.menu} role="menu" aria-label={t("Waveform color")} tabIndex={-1}>
          {CHOICES.map((choice) => (
            <button
              key={choice.value}
              type="button"
              role="menuitemradio"
              aria-checked={choice.value === palette}
              className={styles.item}
              disabled={fromFile}
              onClick={() => choose(choice.value)}
            >
              <span className={styles.tick} aria-hidden>{choice.value === palette ? "✓" : ""}</span>
              {choice.label}
            </button>
          ))}
          {fromFile ? <p className={styles.note}>{t(REASON)}</p> : null}
        </div>
      ) : null}
    </div>
  );
}
