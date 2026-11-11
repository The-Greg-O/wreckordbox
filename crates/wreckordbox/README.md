# wreckordbox (command)

[Crate index](../README.md) · [Service](../wreckordbox-service/README.md)

The headless `wreckordbox` command: a thin client of the Library service. It opens no Library itself. Each subcommand connects to the service for this machine's Library, starting one when none is running, sends one command and prints the answer.

## Start here

[`src/main.rs`](src/main.rs) is the whole command. [`tests/status.rs`](tests/status.rs) runs the built binary against a service on a fixture Library; [`tests/common/mod.rs`](tests/common/mod.rs) is the sandbox those tests share.

## Contracts and safety

- Results go to standard output: text by default, JSON with `--json`.
- Errors go to standard error with exit status 1 (2 for a usage error). With `--json` an error is `{"error": {"code", "message"}}`, with the service's stable code.
- `wreckordbox serve` runs the service in the foreground. The hidden `serve --spawned` is how a client starts one in the background.
- Tests never touch the installed Library: the sandbox clears the environment and sets a private home, runtime directory and `RBXPORT_OPTIONS`.

## Run focused checks

```sh
RBXPORT_TEST=1 cargo test -p wreckordbox
wreckordbox --help
```
