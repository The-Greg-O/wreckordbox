import type { ListTotals } from "@/ipc/types";

/**
 * The open playlist's count and length after its name in the half-width
 * breadcrumb (wbx-6wz; docs/design/screens/mock.html "30 · 2h24m"). Seconds
 * are dropped as in the status bar's tally. A list with no tally (only a
 * playlist has one) says nothing; an empty playlist says only its count.
 */
export function crumbTally(totals: ListTotals | undefined): string {
  if (!totals) return "";
  if (totals.tracks <= 0) return "0";
  const total = Number.isFinite(totals.seconds) && totals.seconds > 0 ? Math.floor(totals.seconds) : 0;
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const length = hours > 0 ? `${hours}h${String(minutes).padStart(2, "0")}m` : `${minutes}m`;
  return `${totals.tracks} · ${length}`;
}
