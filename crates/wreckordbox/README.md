# wreckordbox (command)

[Crate index](../README.md) · [Service](../wreckordbox-service/README.md)

The headless `wreckordbox` command: a thin client of the Library service. It opens no Library itself. Each subcommand connects to the service for this machine's Library, starting one when none is running, sends one command and prints the answer.

## Start here

[`src/main.rs`](src/main.rs) is the whole command. [`tests/status.rs`](tests/status.rs) runs the built binary against a service on a fixture Library; [`tests/common/mod.rs`](tests/common/mod.rs) is the sandbox those tests share.

## Contracts and safety

- Results go to standard output: text by default, JSON with `--json`.
- Errors go to standard error with exit status 1 (2 for a usage error). With `--json` an error is `{"error": {"code", "message"}}`, with the service's stable code.
- `wreckordbox serve` runs the service in the foreground. The hidden `serve --spawned` is how a client starts one in the background.
- `wreckordbox stems separate <TRACK_ID>` is the one slow command: the service runs the separation engine and the command waits, printing progress to standard error. Interrupting it does not stop the separation; running it again waits for the same result.
- `wreckordbox facts record | record-many | show | kinds` and `wreckordbox runs start | finish` record and read Facts in the Sidecar. They never change the Library. `record-many` reads one JSON object per line from standard input: `{"track": "10003", "kind": "genre", "value": "Tech House", "confidence": 0.8}`.
- Tests never run a separation model: the sandbox points the service at the engine's fake backend (`separation-engine/tests/fake_worker.py`) and at a Sidecar in the temporary folder.
- Tests never touch the installed Library: the sandbox clears the environment and sets a private home, runtime directory and `RBXPORT_OPTIONS`.

## Run focused checks

```sh
RBXPORT_TEST=1 cargo test -p wreckordbox
wreckordbox --help
```
