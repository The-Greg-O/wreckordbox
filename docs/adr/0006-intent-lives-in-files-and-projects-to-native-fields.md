# The DJ's intent lives in versioned files and projects to native rekordbox fields

The Tag schema and the Recipe are plain-text files the DJ versions and refines. The Sidecar holds what is known (Facts, Set plans), and Projection turns it into fields rekordbox and players already understand: native MyTags in four categories, Energy in the star rating, Prep colour, and ordinary playlists. Nothing reaches a Stick in a private form, because the player's Track Filter and rekordbox on another machine read only native fields. Changing a file and projecting again rewrites those fields without retagging, and a dry run shows what would change first.

## Considered Options

- **Keep the schema only as rekordbox's four categories, with bulk-edit commands.** Rejected: a genre tree and any fifth dimension would still live in naming conventions, and every remodel would mean editing tags track by track.
- **A schema editor and set builder in the window.** Rejected for now: real interface work for things changed a few times a year, and an agent can edit a file.
- **Move Energy out of the star rating.** Rejected: the stars are filterable on the player, and a measured Intensity carries the finer detail without spending a MyTag category.

## Consequences

- The four projected MyTag categories are chosen for use on the player; anything else known about a track is usable by agents and smart playlists but not in the Track Filter.
- A projection that renames or moves a MyTag must update the intelligent playlists that filter on it, or they would silently empty.
