/**
 * How big a canvas's backing store should be for the space it occupies.
 *
 * A canvas stretched by CSS from a fixed backing store is blurry on a wide
 * window and wasteful on a narrow one, so the store follows the element. Two
 * things stop that being expensive:
 *
 * - the width is quantised, so dragging the window edge reallocates the buffer
 *   a handful of times rather than once per pixel;
 * - for the quantised store only (`backingSize`, the vocal strip), the
 *   device pixel ratio is capped at `MAX_RATIO`.
 *
 * The waveforms (`exactBackingSize`, `previewBackingSize`) are not capped:
 * they are built at the real ratio on any display, 2.5x and 3x included, so
 * one store pixel is one device pixel. Their only limit is allocation:
 * `MAX_WIDTH` device pixels a side. A strip that would need more is drawn at
 * `MAX_WIDTH` and stretched over its element (`coveredSize` is then given the
 * element's size); at 3x that is a strip 5,461 CSS pixels wide.
 *
 * The ratio is the display's times the whole-interface scale
 * (`pixelRatio.ts`), and the limits move with that scale: at 1.25 on a 2×
 * display the element covers 2.5 device pixels per CSS pixel, and at 0.8 on
 * a 1× display it covers 0.8, so the store is neither stretched nor squeezed
 * onto the element at any scale.
 */
import { pixelRatio, pixelZoom } from "./pixelRatio";

/** Reallocate in steps this wide rather than on every pixel of a drag. */
export const STEP = 16;

/**
 * The quantised store's cap (`backingSize`, the vocal strip only): the
 * display's ratio, which the whole-interface scale multiplies. Waveforms are
 * not capped.
 */
export const MAX_RATIO = 2;

/** The quantised store's ratio: `ratio`, within what the window's zoom allows. */
function storeScale(ratio: number): number {
  const zoom = pixelZoom();
  return Math.min(Math.max(finite(ratio, 1), Math.min(1, zoom)), MAX_RATIO * zoom);
}

/** A waveform store's ratio: the real one, anything not a positive number taken as 1. */
function exactScale(ratio: number): number {
  const real = finite(ratio, 1);
  return real > 0 ? real : 1;
}

/**
 * Guards against a runaway layout asking for a buffer megabytes wide.
 *
 * The hardware's own limit, not a budget: 4096 looked generous until the
 * detail waveform, which draws `OVERDRAW` spans at up to 2x, asked for
 * 6,028 device pixels on the reference 1800pt window and got 4,096 stretched
 * across them. A canvas scaled up by 1.47 turns the one-pixel bars into a
 * smear, which reads as the waveform losing its colour. Every engine we ship
 * on takes 16,384 in one dimension.
 */
export const MAX_WIDTH = 16_384;

export interface BackingSize {
  width: number;
  height: number;
}

export function backingSize(
  cssWidth: number,
  cssHeight: number,
  ratio: number = pixelRatio(),
): BackingSize {
  // `Math.max(NaN, 1)` is NaN, and a canvas given a NaN width silently keeps
  // its old buffer — so anything not a real number becomes the floor here.
  const scale = storeScale(ratio);
  const stepped = Math.ceil(Math.max(finite(cssWidth, 1), 1) / STEP) * STEP;
  return {
    width: Math.min(MAX_WIDTH, Math.max(STEP, Math.round(stepped * scale))),
    height: Math.max(1, Math.round(Math.max(finite(cssHeight, 1), 1) * scale)),
  };
}

/**
 * A backing store exactly the device pixels the element covers, unquantised.
 *
 * For a canvas that is redrawn on every size change anyway, where the step
 * saves nothing and costs sharpness: a store 16 CSS pixels wider than its
 * element is squeezed onto it, so every one-pixel bar is resampled across two
 * columns. On the scrolling detail waveform that resampling lands differently
 * after each redraw, which reads as the waveform flickering (#346).
 */
export function exactBackingSize(
  cssWidth: number,
  cssHeight: number,
  ratio: number = pixelRatio(),
): BackingSize {
  const scale = exactScale(ratio);
  return {
    width: Math.min(MAX_WIDTH, wholePoints(cssWidth, scale)),
    height: Math.min(MAX_WIDTH, wholePoints(cssHeight, scale)),
  };
}

/**
 * The device pixels a CSS length is drawn over, at `scale` of them a CSS
 * pixel: a whole number of the display's own points.
 *
 * The engine places a canvas on whole points of the display at the window's
 * zoom, not on whole device pixels: on a 2x display that is two device
 * pixels, so at a 0.7 scale a store 171 wide is drawn over 170 or 172 and
 * every column of it is resampled, while 170 and 172 are drawn exactly, at
 * any position [OBS WebKitGTK, aarch64 Linux, a 2x output: one-pixel stripes
 * in canvases of each size]. On a 1x display a point is a device pixel.
 */
function wholePoints(css: number, scale: number): number {
  const zoom = pixelZoom();
  const points = Math.max(1, Math.round(Math.max(finite(css, 1), 1) * zoom));
  return Math.max(1, Math.round(points * (scale / zoom)));
}

/**
 * A row preview's store: `exactBackingSize` at the ratio itself, uncapped,
 * because the preview's cue badges are drawn at that ratio too.
 */
export function previewBackingSize(cssWidth: number, cssHeight: number, ratio: number = pixelRatio()): BackingSize {
  return exactBackingSize(cssWidth, cssHeight, ratio);
}

/**
 * The CSS box that covers exactly a store's device pixels.
 *
 * An element's size times the ratio is often not a whole number: a 30pt
 * strip at a 1.25 scale covers 37.5 device pixels. The store is whole (38),
 * and shown at the element's own size the engine resamples it onto 37 or 38
 * rows, which blends every row of a waveform into the next [OBS WebKitGTK,
 * aarch64 Linux: screenshots against the canvas's own pixels at each scale].
 * Shown in this box instead, one store pixel is one device pixel; the engine
 * snaps the box's position to whole pixels by itself.
 */
export function coveredSize(store: BackingSize, ratio: number = pixelRatio(), element?: BackingSize): BackingSize {
  const scale = exactScale(ratio);
  // At the allocation limit the store is short of the element's pixels:
  // shown over the element, stretched, rather than leaving it part empty.
  return {
    width: element && store.width >= MAX_WIDTH ? element.width : store.width / scale,
    height: element && store.height >= MAX_WIDTH ? element.height : store.height / scale,
  };
}

function finite(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}
