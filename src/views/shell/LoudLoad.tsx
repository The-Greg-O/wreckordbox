/**
 * The loud load display (wbx-73): a running Stick load fills the Stick view
 * with the waveform of the track being written, a write head sweeping
 * across it while the track is copied, the whole thing moving on the
 * track's beat, behind a very large percentage. docs/design/screens/stick.
 *
 * One replaceable component, fed by the load's progress (`LoadProgress`,
 * coalesced by the store to at most one update per 100 ms) and the track's
 * own waveform and details, read once per track from the Library (never
 * from the Stick, and nothing is written). `LoadProgressPanel`
 * (`StickLoadDisplay.tsx`) picks it or the plain bar by the
 * `view.loudStickLoad` setting.
 *
 * Cheap by construction, so it never slows the load or the window: the
 * waveform is drawn once per track (and on a resize); everything that moves
 * between updates (the sweep and the beat) is a transform animation the
 * compositor runs, with no work on the main thread per frame. A waveform
 * that could not be read or does not exist is said, and a made-up shape is
 * drawn in a muted colour in its place, never the previous track's.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { drawWave, drawBands, waveformKindOf } from "@/canvas/waveform";
import { getBackend } from "@/ipc/client";
import { progressLine, progressPercent, type LoadProgress } from "@/lib/holdToLoad";
import { beatMs, nowDetail, standInBands, sweepMs } from "@/lib/loudLoad";
import { usePixelRatio } from "@/lib/pixelRatio";
import { useShownTheme, useWaveformTheme, waveColoursFrom, wellOf } from "@/lib/waveTheme";
import { shrinkStickLoad, type StickLoad } from "@/store/stickLoads";
import { Bar, DisplaySwitch } from "./StickLoadProgress";
import styles from "./LoudLoad.module.css";

/** What is shown for the track being written. */
type Wave =
  | { state: "reading" }
  | { state: "analysis"; data: Uint8Array }
  | { state: "none" }
  | { state: "failed" };

interface Details { detail: string; bpmX100: number | null; failed: boolean }

/** The track's waveform and details, read once per track; a late answer for an earlier track is dropped. */
function useTrack(id: string | null, palette: ReturnType<typeof useWaveformTheme>["palette"]): { wave: Wave; details: Details | null } {
  const [wave, setWave] = useState<{ for: string; wave: Wave }>({ for: "", wave: { state: "none" } });
  const [details, setDetails] = useState<{ for: string; details: Details } | null>(null);
  const kind = waveformKindOf(palette, false);
  const key = id === null ? "" : `${id}:${kind}`;
  useEffect(() => {
    if (id === null) return;
    let current = true;
    setWave({ for: key, wave: { state: "reading" } });
    void getBackend().then(async (backend) => {
      const read = await Promise.allSettled([backend.trackWaveform(id, kind), backend.trackDetails(id)]);
      if (!current) return;
      const [bytes, info] = read;
      setWave({
        for: key,
        wave: bytes.status === "rejected" ? { state: "failed" } : bytes.value.length === 0 ? { state: "none" } : { state: "analysis", data: bytes.value },
      });
      setDetails({
        for: id,
        details: info.status === "rejected"
          ? { detail: "", bpmX100: null, failed: true }
          : { detail: nowDetail(info.value), bpmX100: info.value.bpmX100 > 0 ? info.value.bpmX100 : null, failed: false },
      });
    });
    return () => { current = false; };
  }, [id, kind, key]);
  return {
    wave: id === null ? { state: "none" } : wave.for === key ? wave.wave : { state: "reading" },
    details: id !== null && details?.for === id ? details.details : null,
  };
}

/** The canvas: the track's waveform, or a muted stand-in; drawn when the track, its bytes, the size or the colours change. */
function WaveCanvas({ wave, seed, palette }: { wave: Wave; seed: string; palette: ReturnType<typeof useWaveformTheme>["palette"] }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const theme = useShownTheme();
  const dpr = usePixelRatio();
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const measure = () => { setSize({ width: canvas.clientWidth, height: canvas.clientHeight }); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(canvas);
    return () => { observer.disconnect(); };
  }, []);
  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas || size.width === 0 || size.height === 0) return;
    const width = Math.max(1, Math.round(size.width * dpr));
    const height = Math.max(1, Math.round(size.height * dpr));
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, width, height);
    if (wave.state === "analysis") {
      drawWave(ctx, wave.data, width, height, palette, false, false, { top: 0, bottom: 0 }, waveColoursFrom(palette, theme, wellOf(canvas)));
      return;
    }
    // No waveform to show: a made-up shape, one muted colour, so it is never taken for the track's own.
    drawBands(ctx, standInBands(seed, 600), width, height, "overview", false);
    ctx.globalCompositeOperation = "source-in";
    ctx.fillStyle = getComputedStyle(canvas).color;
    ctx.fillRect(0, 0, width, height);
    ctx.globalCompositeOperation = "source-over";
  }, [wave, seed, palette, theme, dpr, size]);
  return <canvas ref={ref} className={styles.wave} data-standin={wave.state !== "analysis" || undefined} aria-hidden />;
}

/** The write head: sweeps across the track over about as long as recent tracks took, then waits at the end. */
function useSweep(head: React.RefObject<HTMLDivElement | null>, step: string, running: boolean): void {
  const seen = useRef<{ step: string; at: number; took: number[] }>({ step: "", at: 0, took: [] });
  useEffect(() => {
    const el = head.current;
    if (!el) return;
    const now = performance.now();
    const memory = seen.current;
    if (memory.step !== "" && memory.step !== step) memory.took = [...memory.took, now - memory.at].slice(-8);
    memory.step = step;
    memory.at = now;
    // Reduced motion: no sweep; the waveform is shown whole.
    if (!running || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      el.getAnimations().forEach((a) => { a.cancel(); });
      el.style.transform = "translateX(100%)";
      return;
    }
    el.style.transform = "";
    const animation = el.animate(
      [{ transform: "translateX(0%)" }, { transform: "translateX(96%)" }],
      { duration: sweepMs(memory.took), easing: "cubic-bezier(0.2, 0.6, 0.3, 1)", fill: "forwards" },
    );
    return () => { animation.cancel(); };
  }, [head, step, running]);
}

export function LoudLoad({ load, progress }: { load: StickLoad; progress: LoadProgress | null }) {
  const { palette } = useWaveformTheme();
  const track = progress?.track ?? null;
  const { wave, details } = useTrack(track?.id ?? null, palette);
  const shown: Wave = track === null ? { state: "none" } : track.id === null ? { state: "none" } : wave;
  const percent = progressPercent(progress);
  const head = useRef<HTMLDivElement>(null);
  const copying = progress !== null && track !== null && progress.stage !== "done";
  useSweep(head, `${track?.id ?? ""}\u0000${track?.title ?? ""}\u0000${progress?.done ?? -1}`, copying);
  const beat = beatMs(details?.bpmX100 ?? null);
  const why = shown.state === "failed" ? "Its waveform could not be read."
    : shown.state === "none" && track !== null ? "No waveform: this track is not analysed."
      : null;
  return (
    <section className={styles.loud} data-testid="stick-progress" aria-label={`Loading ${load.name}`}>
      <div
        className={styles.stage}
        data-testid="stick-loud"
        data-track={track?.id ?? undefined}
        data-wave={track === null ? undefined : shown.state}
        style={{ "--beat-ms": `${beat}ms` } as React.CSSProperties}
      >
        <div className={styles.waveBox}>
          <WaveCanvas wave={shown} seed={track?.title ?? load.name} palette={palette} />
          <div ref={head} className={styles.head} aria-hidden />
        </div>
        <header className={styles.top}>
          <div>
            <h2 className={styles.title}>Loading {load.name}</h2>
            <p className={styles.line} role="status" aria-live="polite">{progressLine(progress)}</p>
          </div>
          <div className={styles.actions}>
            <DisplaySwitch loud />
            <button type="button" className={styles.shrink} onClick={() => { shrinkStickLoad(load.id); }}>
              <kbd>Esc</kbd> shrink to the status line and keep working
            </button>
          </div>
        </header>
        <p className={styles.percent} aria-hidden>{percent === null ? "…" : `${percent}%`}</p>
        <div className={styles.now}>
          <p className={styles.track} data-testid="stick-loud-track">{track?.title ?? (progress?.stage === "done" ? "Done" : "Getting ready")}</p>
          <p className={styles.detail} data-testid="stick-loud-detail">
            {details?.failed ? "Its details could not be read." : details?.detail ?? ""}
            {why !== null ? <span className={styles.why}>{why}</span> : null}
          </p>
          <p className={styles.note}>Do not remove {load.name} while it is loading.</p>
        </div>
        <div className={styles.bar}><Bar progress={progress} label={`Loading ${load.name}`} /></div>
      </div>
    </section>
  );
}
