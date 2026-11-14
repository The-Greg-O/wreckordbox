import { describe, expect, it } from "vitest";

import { fromServiceStatus } from "./stemsStatus";

describe("fromServiceStatus", () => {
  it("reads the service's answer", () => {
    expect(fromServiceStatus({ state: "stale", made_at: 5, separating: "7", unavailable: null, track: {} }))
      .toEqual({ state: "stale", madeAt: 5, separating: "7", unavailable: null });
  });
  it("treats anything else as no Stems and keeps a reason", () => {
    expect(fromServiceStatus({ state: "odd", unavailable: { code: "stick_protected", message: "locked" } }))
      .toEqual({ state: "none", madeAt: null, separating: null, unavailable: { code: "stick_protected", message: "locked" } });
  });
});
