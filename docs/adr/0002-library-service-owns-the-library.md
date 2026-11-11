# A background service owns the Library; the window and the CLI are its clients

The desktop window and the headless CLI are equal front ends: anything a DJ can do in one can be done in the other. To keep exactly one writer, a background Library service owns the Library, and both front ends talk to it rather than opening the Library themselves. We accepted restructuring upstream's app, which keeps the Library inside the window's process, because agent edits must appear live in the window and two processes writing one SQLite file is a corruption risk.

## Considered Options

- **The CLI writes the Library files directly and the window reloads.** Rejected: two writers, and agent edits bypass the window's undo history.
- **Whoever is running owns it** (the CLI hands commands to the window when it is open, and writes directly when it is not). Rejected in favour of one model with no second code path, since staying mergeable with upstream is not a goal (ADR 0003).
