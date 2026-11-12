import { keyLight } from "@/lib/keyLights";
import { useTranslation } from "@/i18n";
import styles from "./LibraryKey.module.css";

/** Display only: the containing cell still owns the existing Library edit. */
export function LibraryKey({ stored, loaded, text }: { stored: string; loaded: string | null; text: string }) {
  const t = useTranslation();
  const light = keyLight(loaded, stored);
  const hint = light === "match" ? t("Harmonic match") : light === "boost" ? t("Energy boost (+2)") : null;
  return <span className={styles.key} data-light={light} title={hint ?? undefined}
    aria-label={hint ? `${text}: ${hint}` : undefined}>{text}</span>;
}
