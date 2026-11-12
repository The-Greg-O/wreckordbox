/** Canvas drawing entry point — lazily chunked, kept off the cold-start path. */
export {
  bandStops, drawBands, drawColumns, drawPcmWave, drawPreview, drawPreviewCues, drawPreviewMemoryCues, drawWave, previewClickMs, ramp, renderPreview,
  MEASURED_COLOURS, strideOf, themeWaveColours, waveformKindOf, WaveformCache,
} from "./waveform";
export type { HalfWaveform, PreviewCue, RenderedWaveform, WaveBand, WaveColours, WavePalette } from "./waveform";
