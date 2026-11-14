# Loading a Stick from the command line

[Documentation](../README.md) · [USB export](usb-export.md)

The `wreckordbox` command can load chosen playlists onto a mounted Stick,
with the same export the window uses. A load is a sync: the Stick ends up
holding the chosen playlists and no other playlist of your Library (tracks
put on it on their own with Export Track stay). On a Spare stick (one with
an empty `.rbxport-disposable` file at its root) the tracks' Native stems
are written too; a Gig stick never gets them.

## The three steps

```sh
wreckordbox stick list
wreckordbox stick dry-run /run/media/you/FRIDAY 1234 5678
wreckordbox stick load /run/media/you/FRIDAY 1234 5678 --plan <plan id>
```

1. `stick list` shows each mounted Stick: Gig or Spare, protected or
   unlocked, when it was last loaded and how much room it has.
2. `stick dry-run` shows everything the load would do (tracks added,
   changed field by field, given new audio and removed; playlists created,
   reordered and removed; Native stems; the room before and after) and
   writes nothing. It prints a plan id.
3. `stick load ... --plan <id>` does exactly what that dry run reported,
   naming each track as it is written, then reads the Stick back and says
   what was written, what was not, and what needed no change.

A load is refused, with nothing written, when:

- the Stick is protected (see below);
- the plan id is missing, already used, more than ten minutes old, or from
  a dry run of other playlists or another Stick;
- anything the load would write has changed since the dry run: the Stick,
  the Library, a track's audio file, a set of Stems (a separation that
  finished in between), the playlists or the changes themselves. The
  message says what changed; make a new dry run;
- it would not fit, saying by how many bytes;
- the Stick's database was left in WAL mode by rekordbox or a player (the
  dry run then gives no plan id and says why), or an earlier write did not
  finish (settle it in the window first).

If the Library service is stopped during a load (for example because the
window opens and takes the Library over), the load stops where it safely
can and still reports what is and is not on the Stick.

## Who can load a Stick

Every Stick is protected until it is unlocked, and an unlock lasts until
the Stick is ejected, removed, put back or locked again. Unlocking is the
guarantee; the plan id is only the confirmation that the load is the one
you looked at.

The command line cannot tell an agent from you, and it does not enforce
any rule about who asked for what. What an agent working for you can do:

- list Sticks and dry-run playlists at any time, also while a Stick is
  protected;
- only after you have explicitly asked for that Stick, by name, to be
  unlocked: run `wreckordbox stick unlock PATH` for a Spare stick, or
  `wreckordbox stick unlock PATH --name NAME` for a Gig stick;
- then make a fresh dry run and run `wreckordbox stick load PATH
  PLAYLIST_IDS... --plan ID` (a Spare stick also gets the available Native
  stems; a Gig stick does not);
- `wreckordbox stick lock PATH` protects the Stick again.

Nothing stops an agent that ignores these rules from typing a Stick's name
itself: keep a Stick you do not want written to locked, and eject it when
you are done.

## After removing and re-importing a track

Re-importing gives a track a new Library ID. Its earlier Facts and Stems
remain attached to the removed ID until a producer observes the new track.
`facts show --check-audio`, `stems show`, and Stick dry runs only read:
looking does not reattach anything. The new track initially shows no Facts
or Stems. A Spare-stick dry run and load omit its Native stems; a load can
still export the ordinary audio.

Run `wreckordbox stems separate NEW_TRACK_ID`. If the audio matches and all
four saved Stem files are intact, this reattaches and reuses the set without
running the separation engine again. Facts then appear under the new track
and the next dry run includes its Native stems. If the audio or saved files
have changed, separation may need to run again. Make a fresh dry run before
confirming the next load.
