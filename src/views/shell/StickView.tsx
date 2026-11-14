import type { ReactNode } from "react";

import { EmptyView } from "./EmptyView";

/**
 * The Stick view's place in the working area. `AppBody` hands it the view
 * itself (`StickLoadView`, wbx-71: what a load will do); shown without one
 * (the shell on its own in a test) it says so.
 */
export function StickView({ stick }: { stick?: ReactNode | undefined }) {
  if (stick) return <>{stick}</>;
  return <EmptyView title="Stick" note="Nothing to show here." />;
}
