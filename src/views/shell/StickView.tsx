import type { ReactNode } from "react";

import { EmptyView } from "./EmptyView";

/**
 * The Stick view: what a load will do, and the hold-to-load button. Not
 * built yet; this is where it goes (`docs/design/screens/stick`). Until
 * then, a Stick selected in the tree shows its existing panel here
 * (`stick`), from which it is loaded as before.
 */
export function StickView({ stick }: { stick?: ReactNode | undefined }) {
  if (stick) return <>{stick}</>;
  return <EmptyView title="Stick" note="Select a Stick in the tree to load it. Nothing is here yet." />;
}
