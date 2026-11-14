/**
 * The shell's views whose code is loaded on demand (wbx-45.13): each is its
 * own chunk, kept off the window's start so the first paint does not wait
 * for it. `AppBody` draws these in place of the views themselves and calls
 * `preloadShellViews` once the shell has painted, so a first switch finds
 * the code already there.
 */
import type { ComponentProps } from "react";

import type { IntakeView } from "./IntakeView";
import type { ReviewView } from "./ReviewView";
import type { StickLoadView } from "./StickLoadView";
import { onDemand } from "./onDemand";

export const IntakeOnDemand = onDemand<ComponentProps<typeof IntakeView>>(
  "Intake",
  () => import("./IntakeView").then((m) => m.IntakeView),
);

export const StickLoadOnDemand = onDemand<ComponentProps<typeof StickLoadView>>(
  "Stick",
  () => import("./StickLoadView").then((m) => m.StickLoadView),
);

export const ReviewOnDemand = onDemand<ComponentProps<typeof ReviewView>>(
  "Review",
  () => import("./ReviewView").then((m) => m.ReviewView),
);

/** Fetches every on-demand view's code; a failure is shown when that view is drawn. */
export function preloadShellViews(): void {
  for (const view of [IntakeOnDemand, ReviewOnDemand, StickLoadOnDemand]) view.preload().catch(() => {});
}
