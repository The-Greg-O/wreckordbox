# Separation engine

The Python worker that splits a track into Stems. Wreckordbox runs it as a
child process and talks to it over standard input and output with JSON lines;
it is not linked into any Rust crate. The wire contract is
[`docs/PROTOCOL.md`](docs/PROTOCOL.md).

## Origin

Copied as it stands from the `engine/` folder (and `docs/PROTOCOL.md`) of
Dubplate, the fork owner's earlier macOS stem-splitter project
(`The-Greg-O/dubplate`, private, commit `86ca5c7`). Nothing in the Python
source, the tests, `pyproject.toml` or `uv.lock` was changed. The Swift app,
its build scripts and its Rust core were left behind.

The Python package keeps its original name, `stemsplitter_engine`. Comments
and defaults still describe the macOS app (for example the default
`--model-dir` and the homebrew `PATH` entries); see "Linux notes".

## Licence

The Dubplate repository states no licence: it has no `LICENSE` file and no
licence field in `pyproject.toml`. The code is by the same author as this
fork and is included here on that basis, under this repository's
GPL-2.0-or-later like everything else (see [`LICENSING.md`](../LICENSING.md)).
If the author wants different terms for the engine, this paragraph is the
place to say so.

The real backend depends on third-party packages that are installed, not
vendored: [`audio-separator`](https://github.com/nomadkaraoke/python-audio-separator)
(MIT) and what it pulls in (PyTorch, ONNX Runtime, librosa and others).
Model checkpoints are downloaded at run time and are not part of this
repository; each carries its own terms (the standard 4-stem model is
`htdemucs_ft`, Demucs v4, MIT).

## Tests

Requires [`uv`](https://docs.astral.sh/uv/) (`mise use uv` or
`mise exec uv -- ...`). uv fetches the pinned Python (3.13; the package
requires `>=3.11,<3.14`).

```sh
cd separation-engine
uv sync                                         # base deps + pytest, about 100 MB
uv run --no-sync pytest -q -m "not integration" # hermetic: fake backend, no model
```

The hermetic tests use the fake backends defined in the test files and need
neither the real backend nor a network.

Do not run the tests marked `integration` on Linux as they stand. They
download several GB of models into a hard-coded macOS path
(`~/Library/Application Support/Splittr/models`).

## A real separation

```sh
cd separation-engine
uv sync --extra full        # audio-separator, PyTorch, ONNX Runtime
.venv/bin/python -m stemsplitter_engine --model-dir ~/.cache/wreckordbox/separation/models
```

The worker prints one `hello` line, then reads commands from standard input:

```json
{"cmd":"separate","id":"1","input":"/abs/track.mp3","mode":"four_stem","workspace":"/abs/out-dir","quality":"standard"}
{"cmd":"shutdown"}
```

It answers with `progress` lines and one `done` (stem paths, `source_samples`,
`sample_rate`, `model`) or `error`. Stems are WAV files in `workspace`.

## Linux notes

- Always pass `--model-dir`. The default is a macOS path under `~/Library`,
  which on this fork's target machine is where the rekordbox Library lives.
- The worker also writes beside the model directory: `staging/` (model output
  before it is moved to the workspace) and `bin/ffmpeg` (a link to the static
  ffmpeg from `imageio-ffmpeg`, put first on the worker's `PATH`). Keep the
  model directory's parent somewhere disposable and outside the repository.
- `ffprobe` is taken from the system `PATH` for the source duration; without
  it the engine falls back to libsndfile.
- Models, the virtual environment and caches are never committed
  (`.gitignore` here covers `.venv/` and Python caches).

## Measured on ARM Linux

The real backend installs from the unchanged `uv.lock` and runs on aarch64
Linux (audio-separator 0.44.2, PyTorch 2.13.0, CPU only). One run, machine
otherwise idle, through the worker protocol above:

| | |
|---|---|
| Machine | Apple M2, 8 cores, 15 GB RAM, Asahi Linux aarch64 |
| Request | `four_stem`, `quality: standard` (model `htdemucs_ft`) |
| Track | stereo 44.1 kHz MP3, 304.3 s |
| Wall time | 1358 s, which is 4.46 times the track's length |
| Peak memory | 4.4 GB resident |
| Cores used | 5.0 of 8 on average |
| Sample count | source and all four stems: 13,418,790 frames each |

`htdemucs_ft` is a bag of four models, one fine-tuned per stem, and the
library runs each twice with a random time shift (`shifts=2`) and averages:
8 passes over the track, each in overlapping chunks of about 7.8 s. The engine
exposes no faster setting (`quality` is `standard` or the slower `max`); the
library has them (`shifts=1`, or the single-model `htdemucs`), unmeasured.

The first run downloads the model (about 320 MB) into `--model-dir`. Stems
from an MP3 source are written as 16-bit PCM WAV. The `separating` progress
fraction is a clock-based estimate and can sit still for most of a first run.
