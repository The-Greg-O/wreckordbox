/**
 * Whether a track has Stems (wbx-67), in the view's terms. The service's
 * `stems.status` only reads: it never marks Stems stale or makes a
 * Sidecar, so the window may ask each time a track is loaded.
 */
export type StemsState = "none" | "current" | "stale";

export interface StemsStatus {
  state: StemsState;
  /** When the Stems the state describes were made (ms since the epoch). */
  madeAt: number | null;
  /** The track any client is separating now. */
  separating: string | null;
  /** Why the state could not be read in full (audio missing, Sidecar refused). */
  unavailable: { code: string; message: string } | null;
}

/** The service's snake_case answer as the view reads it; unknown fields ignored. */
export function fromServiceStatus(raw: unknown): StemsStatus {
  const value = (raw ?? {}) as Record<string, unknown>;
  const state = value.state === "current" || value.state === "stale" ? value.state : "none";
  const unavailable = value.unavailable as { code?: unknown; message?: unknown } | null | undefined;
  return {
    state,
    madeAt: typeof value.made_at === "number" ? value.made_at : null,
    separating: typeof value.separating === "string" ? value.separating : null,
    unavailable: unavailable && typeof unavailable.message === "string"
      ? { code: typeof unavailable.code === "string" ? unavailable.code : "", message: unavailable.message }
      : null,
  };
}
