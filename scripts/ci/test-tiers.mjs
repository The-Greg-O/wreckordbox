import { validationKeys } from "./validation-plan.mjs";

/**
 * Which validation lanes run when. `docs/development/test-tiers.md` explains
 * each tier; `test-tiers.test.mjs` keeps the two and the lane planner in step,
 * so adding a lane without deciding when it runs fails a test.
 *
 * `active` tiers are wired into workflows today. `planned` tiers are agreed
 * but not built, and must not be relied on for coverage yet.
 */
export const tiers = [
  {
    id: "local",
    status: "active",
    trigger: "while working, by a person or an agent",
    lanes: [],
    selection: "scoped commands only (cargo test -p <crate>, vitest <paths>, one e2e spec)",
  },
  {
    id: "pull-request",
    status: "active",
    trigger: "each push to a pull request; a newer push cancels the older run",
    lanes: validationKeys,
    selection: "path plan from scripts/ci/validation-plan.mjs, affected Rust packages only",
  },
  {
    id: "integration",
    status: "active",
    trigger: "push to main; runs are not cancelled",
    lanes: validationKeys,
    selection: "every lane on Linux",
  },
  {
    id: "nightly",
    status: "planned",
    trigger: "scheduled, and only when main changed since the last green run",
    lanes: [],
    selection: "the cross-platform lanes",
  },
  {
    id: "release",
    status: "active",
    trigger: "manual workflow_dispatch",
    lanes: validationKeys,
    selection: "every lane on Linux; no publishing",
  },
];

/** Lanes that an `active` tier runs unconditionally, not by path. */
export function lanesAlwaysRun() {
  return new Set(
    tiers
      .filter(tier => tier.status === "active" && tier.selection.startsWith("every lane"))
      .flatMap(tier => tier.lanes),
  );
}
