/**
 * The DJ's theme file has mistakes: the status line says so, with the first
 * one and its line, and every one in the tooltip. What a mistake would have
 * set was left out by the app shell (`src-tauri/src/theme_file.rs`) and the
 * rest of the file applies. Put away, it stays away until the mistakes
 * themselves change: the same ones arriving with another colour (a valid
 * key saved, an Omarchy switch) do not bring it back, and once the file is
 * fixed the dismissal is forgotten.
 */
import { useSyncExternalStore } from "react";

import { useTranslation } from "@/i18n";
import { shownTheme, subscribeShownTheme, type ThemeProblem } from "@/lib/themeRoles";
import styles from "./ThemeFileStatus.module.css";

const NONE: ThemeProblem[] = [];

/** The mistakes by what they say, not by which event carried them. */
const keyOf = (problems: readonly ThemeProblem[]): string =>
  problems.map((problem) => `${problem.line}\u0000${problem.message}`).join("\u0001");

// Shared by both layouts' status lines and kept across a switch between them.
let dismissed: string | null = null;
const listeners = new Set<() => void>();
const subscribeDismissed = (listener: () => void) => {
  listeners.add(listener);
  return () => void listeners.delete(listener);
};
function dismiss(key: string | null): void {
  dismissed = key;
  for (const listener of [...listeners]) listener();
}
subscribeShownTheme(() => {
  // Fixed: a mistake made again later is news again.
  if (dismissed !== null && (shownTheme().problems ?? NONE).length === 0) dismiss(null);
});

export function ThemeFileStatus() {
  const t = useTranslation();
  const problems = useSyncExternalStore(subscribeShownTheme, () => shownTheme().problems ?? NONE);
  const hidden = useSyncExternalStore(subscribeDismissed, () => dismissed);
  const key = keyOf(problems);
  if (problems.length === 0 || hidden === key) return null;
  const lines = problems.map((problem) => `${t("line {line}", { line: problem.line })}: ${problem.message}`);
  return (
    <span className={styles.item} role="status" aria-label={t("Theme file")} title={lines.join("\n")} data-i18n-ignore>
      <span className={styles.text}>
        {t("Theme file")} · {lines[0]}
        {problems.length > 1 ? ` (+${problems.length - 1})` : ""}
      </span>
      <button type="button" className={styles.dismiss} aria-label={t("Dismiss")} onClick={() => dismiss(key)}>×</button>
    </span>
  );
}
