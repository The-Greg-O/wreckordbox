import { EmptyView } from "./EmptyView";

/**
 * The Stick view: what a load will do, and the hold-to-load button. Not
 * built yet; this is where it goes (`docs/design/screens/stick`). Until
 * then a Stick is loaded as before: select it in the tree, in the Library
 * view.
 */
export function StickView() {
  return <EmptyView title="Stick" note="Loading a Stick will happen here. Nothing is here yet: until then, select the Stick in the tree from the Library view." />;
}
