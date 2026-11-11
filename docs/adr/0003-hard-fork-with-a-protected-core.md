# Hard fork of rbxport, with the player-format crates kept close to upstream

Wreckordbox is a hard fork: the app, the window and everything new are restructured freely, with its own name and identity, and upstream changes are cherry-picked rather than merged. The exception is the crates that write what players and rekordbox read (`rbl-pdb`, `rbl-onelibrary`, `rbl-export`, `rbl-anlz`, `rbl-db`), which stay as close to upstream as we can manage, with our additions beside them rather than inside them. Upstream's hardware-compatibility fixes land there and are the part we are least able to verify ourselves, so cherry-picks into those crates must keep applying cleanly.

## Consequences

- Linux on Omarchy is the only platform shaped and tested. macOS builds are kept compiling when cheap, with no promise.
- The fork remains a derivative of rbxport and stays GPL-2.0-or-later when shared. Writing AZ-MODS's stems layout from its specification does not change that; copying AZ-MODS code in would make the combined work GPL-3.0.
