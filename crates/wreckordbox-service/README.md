# wreckordbox-service

[Crate index](../README.md) · [ADR 0002](../../docs/adr/0002-library-service-owns-the-library.md)

The Library service: one process owns the Library, and the window and the `wreckordbox` command are its clients. This crate holds both ends, the service and the client, and the protocol between them. It does not depend on the desktop framework and needs no display.

## Start here

Read the crate docs in [`src/lib.rs`](src/lib.rs), then [`src/protocol.rs`](src/protocol.rs) for what goes over the socket. [`tests/in_process.rs`](tests/in_process.rs) hosts a service in the test's own process; [`../wreckordbox/tests/status.rs`](../wreckordbox/tests/status.rs) drives the real command against one.

## Code map

| File in `src/` | Responsibility |
| --- | --- |
| [`lib.rs`](src/lib.rs) | `Config`: where the socket goes and how long an idle service lives. Finding the Library. |
| [`protocol.rs`](src/protocol.rs) | Requests, responses, events, error codes, command names. |
| [`library.rs`](src/library.rs) | What the service owns of the Library: an `rbl_library::LibraryCore` (index, generation, the single edit path). Queries and edits. |
| [`server.rs`](src/server.rs) | `Service`: the Library's lock, the open Library, the socket, command dispatch, events. |
| [`client.rs`](src/client.rs) | `Client`: connect, call, subscribe; starting a service on demand. |

## Contracts and safety

- Transport is a Unix domain socket, one JSON object per line. A request is `{"id", "command", "args"}`; a response is `{"id", "result"}` or `{"id", "error": {"code", "message"}}`; an event, sent only to connections that sent `subscribe`, is `{"event", "data"}`. Error codes are stable; messages are not.
- One service per Library. A service holds a lock on `.wreckordbox-service.lock` beside `master.db` for its whole life, so a second service for the same Library refuses to start whatever runtime directory it was given. The kernel releases the lock when the process ends.
- The socket is `<runtime dir>/<hash of the master.db path>.sock`. The runtime directory is `$XDG_RUNTIME_DIR/wreckordbox` unless `WRECKORDBOX_RUNTIME_DIR` names another; it is made with mode `0700` and the socket with `0600`.
- A client that finds no service starts one (`Client::connect_or_start`) and waits for it. A service started that way stops after five minutes with no client connected; `WRECKORDBOX_IDLE_EXIT_SECS` changes that.
- The Library is found with `rbl_db::detect`, exactly as the window finds it, so `rbl-db`'s write guards and its `RBXPORT_OPTIONS` test hook apply unchanged.
- Every edit goes through `rbl_library::LibraryCore::edit`, the path the window's edits take, so `rbl-db`'s write guards apply to it and nothing here opens the Library for writing itself. A guard's refusal is the `write_refused` error carrying the guard's own message.
- After every change to the Library, every subscribed connection is sent `{"event": "library_changed", "data": {"kind": "library" | "tag_list", "generation": N}}`: what the window's own change events carry. The service hears of changes from the Library core (`LibraryCore::watch`), so this covers a change the hosting window made itself, and events go out in the order the changes happened. A client that leaves an event unread for two seconds is disconnected.
- The window hosts the service while it is open (`Service::host`): the service runs on the window's own `LibraryCore`, so there is one in-memory Library and one generation, and a client's edit borrows the window's `rbl_library::Host`. Until the window has read the Library, commands answer `library_loading`.
- A window that starts while a service is running by itself asks it to stop (the `stop` command) and takes its place, so the window is the one writer while it is open; its other commands still write in its own process. A service the window hosts answers `stop` with `stop_refused`, and so a second window on the same Library cannot become a writer.
- The process hosting a service calls it with `Service::call`, through the same dispatch as a socket client's command. `Service::events` gives it the changes clients made, which it has not seen; its own changes are not repeated to it. `Service::stopped` is how it learns the service has gone.
- Add a command in one place: a name in `protocol::commands` and an arm in `Shared::dispatch`.

## Run focused checks

From the repository root:

```sh
RBXPORT_TEST=1 cargo test -p wreckordbox-service -p wreckordbox
cargo clippy -p wreckordbox-service -p wreckordbox --all-targets -- -D warnings
```
