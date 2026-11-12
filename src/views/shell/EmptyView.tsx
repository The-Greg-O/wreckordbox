import { useTranslation } from "@/i18n";
import styles from "./ViewShell.module.css";

/**
 * A view of the shell that has nothing in it yet: its name and one line on
 * what it is for. Also what a built view shows when its data does not exist
 * (spec #45: "absent data shows as an empty state, not an error").
 */
export function EmptyView({ title, note }: { title: string; note: string }) {
  const t = useTranslation();
  return (
    <div className={styles.empty} data-testid="shell-empty">
      <h2 className={styles.emptyTitle}>{t(title)}</h2>
      <p className={styles.emptyNote}>{t(note)}</p>
    </div>
  );
}
