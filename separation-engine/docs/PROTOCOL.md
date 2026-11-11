# Engine protocol v1

Contract between the Swift app and the Python worker (`python -m stemsplitter_engine`).
Newline-delimited JSON, UTF-8. Worker stdout carries protocol lines ONLY (all logging
goes to stderr); every line is flushed immediately. The app writes one command object
per line to the worker's stdin.

## Startup

Worker emits exactly one `hello` before anything else:

```json
{"type":"hello","protocol":1,"engine_version":"0.1.0","default_models":{"two_stem":"<ckpt>","four_stem":"<ckpt>"}}
```

The app must verify `protocol == 1`.

## Commands (app → worker)

Commands use the `cmd` field as discriminator (events from the worker use `type`), e.g.:

```json
{"cmd":"separate","id":"5A2C…","input":"/abs/track.mp3","mode":"two_stem","workspace":"/abs/job-dir"}
```

| cmd | fields | reply |
|---|---|---|
| `ping` | `id` | `pong` |
| `separate` | `id`, `input` (abs path), `mode` (`"two_stem"`\|`"four_stem"`), `workspace` (abs dir for stem WAVs), `quality` (`"standard"`\|`"max"`, optional; default `"standard"`) | `progress`* then `done`/`error` |
| `export` | `id`, `stems` (`[{name,path,window,gain}]`; each `window` is `{start_sample,end_sample}` or absent/`null` for full track; each `gain` is a linear float or absent/`null` for `1.0`), `source` (abs path), `source_samples` (int), `span` (`{start_sample,end_sample}`, required — output range on the source timeline), `format` (`"aiff"`\|`"wav"`), `out_dir`, `suffixes` (`{stem_name: "Suffix"}`), `combine_suffix` (`string` or `null`, optional; default `null`) | `progress`* then `done`/`error` |
| `prefetch` | `id` | `progress`* then `done`/`error` — downloads/loads default models |
| `cancel` | `id`, `target_id` | no direct reply; target job errors with category `cancelled` (best-effort, at stage boundaries) |
| `shutdown` | — | worker exits 0 |

Sequencing: `ping`, `cancel`, `shutdown` are handled immediately on the read thread.
`separate`/`export`/`prefetch` run one at a time on a single job thread; submitting a
job while one is running yields `error` with category `busy`.

## Events (worker → app)

```json
{"type":"pong","id":"..."}
{"type":"progress","id":"...","stage":"loading_model|decoding|separating|writing|exporting|downloading","fraction":0.42}
{"type":"done","id":"...","result":{...}}
{"type":"error","id":"..."|null,"category":"decode|model|internal|cancelled|busy|protocol","message":"..."}
```

Malformed JSON or unknown `cmd` → `error` with category `protocol` (`id` null if unparseable).

`done.result` shapes:
- separate: `{"stems":[{"name":"vocals","path":"/abs.wav"},...],"source_samples":int,"sample_rate":int,"model":"<ckpt>"}`
- export: `{"files":["/abs/out.aiff",...]}`
- prefetch: `{"models":["<ckpt>",...]}`

Stem names: `two_stem` → `vocals`, `instrumental`; `four_stem` → `vocals`, `drums`, `bass`, `other`.

## Separation quality

`separate.quality` selects how hard the engine works; both values yield the same
stem names for the mode. `done.result.model` describes the actual pipeline used
(a single checkpoint for standard, an `ensemble(...)` / `cascade(...)` string for max).

- `standard` (default): one checkpoint, one pass. `two_stem` → BS-RoFormer ep317;
  `four_stem` → htdemucs_ft. Behavior is unchanged from prior versions.
- `max`: trades wall time for quality.
  - `two_stem`: an ensemble of two differently-architected vocal/instrumental
    models (BS-RoFormer ep317 + the best installed Mel-Band RoFormer vocals
    checkpoint); their vocals are averaged pairwise and their instrumentals are
    averaged pairwise. `model` = `ensemble(model_a + model_b)`.
  - `four_stem`: a cascade — pass 1 removes vocals with BS-RoFormer ep317, pass 2
    splits the vocal-free instrumental into drums/bass with htdemucs_ft, and
    `other` is the exact residual `instrumental − drums − bass`, so the four
    stems sum back to pass 1's vocals + instrumental. `model` describes the cascade.

  Progress is estimated across all passes; cancellation is honored between passes.

## Export behavior (engine-side invariants)

- Float32 pipeline throughout; output encoded PCM 24-bit (AIFF or WAV).
- Length invariant: every loaded stem MUST have exactly `source_samples` frames, else `error` category `internal`.
- Windowing: each stem has an optional per-stem `window` (`{start_sample,end_sample}`, absent/`null` = full track), validated as `0 <= start_sample < end_sample <= source_samples`. A stem is SILENT outside its window; inside it carries the source samples, aligned on the original timeline. A window that does not intersect the `span` yields pure silence (allowed, not an error). The old top-level `trim` field is gone.
- Span: the required `span` (`{start_sample,end_sample}`, validated as `0 <= start_sample < end_sample <= source_samples`) is the output range on the source timeline. EVERY output (per-stem files and the combined mix) has exactly `end_sample - start_sample` frames.
- Per-stem gain: each stem carries an optional linear `gain` (default `1.0`) applied to its built (windowed) buffer BEFORE the shared anti-clip pass — in both per-stem and combined exports. A gain can therefore be what pushes the mix into the anti-clip guard.
- Anti-clip: if any stem's built (windowed, gained) buffer peaks > 0.999, ONE shared gain (0.999 / max peak across all stems) is applied to every stem equally. Never per-stem normalization — the relative levels set by the per-stem gains are preserved exactly.
- Output filename: `<source basename> (<suffix>).<ext>`, `/` and `:` sanitized out of the suffix/basename.
- Tags: title/artist/album copied from source when readable (ID3, MP4, Vorbis, ID3-in-AIFF/WAV); exported title = source title (fallback: source basename) + ` (<suffix>)`.
- Combined mode: when `combine_suffix` is a string the listed stems are summed into a single output file (using `combine_suffix` as the name/tag suffix, `suffixes` ignored) whose length, anti-clip, and tag rules are identical to per-stem exports; `done.result.files` then holds that one path.
