/**
 * The DJ's theme file had mistakes: each is listed with its line, so it can
 * be fixed. What the mistakes would have set was left out by the app shell
 * (`src-tauri/src/theme_file.rs`), and the rest of the file applies. The
 * notice can be put away; it comes back when the theme is next announced
 * with problems, which is when the file is next saved.
 */
import { useState, useSyncExternalStore } from "react";

import { useTranslation } from "@/i18n";
import { shownTheme, subscribeShownTheme, type ThemeProblem } from "@/lib/themeRoles";
import styles from "./ThemeFileNotice.module.css";

const NONE: ThemeProblem[] = [];

export function ThemeFileNotice() {
  const t = useTranslation();
  const problems = useSyncExternalStore(subscribeShownTheme, () => shownTheme().problems ?? NONE);
  const [dismissed, setDismissed] = useState<ThemeProblem[] | null>(null);
  if (problems.length === 0 || dismissed === problems) return null;
  return (
    <div role="status" aria-label={t("Theme file")} className={styles.notice}>
      <div className={styles.head}>
        <span className={styles.title}>{t("Theme file")}</span>
        <button type="button" className={styles.dismiss} aria-label={t("Dismiss")} onClick={() => setDismissed(problems)}>
          ×
        </button>
      </div>
      <ul className={styles.list} data-i18n-ignore>
        {problems.map((problem, at) => (
          <li key={`${problem.line}-${at}`}>
            <span className={styles.line}>{t("line {line}", { line: problem.line })}</span> {problem.message}
          </li>
        ))}
      </ul>
    </div>
  );
}
