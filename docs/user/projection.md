# Projection: Energy and Prep colour

Projection writes what is known about your tracks (their Facts) into the
fields rekordbox and the players read: Energy into the star rating, and
Prep colour into the colour label. It runs only when you, or an agent, ask
for it:

```sh
wreckordbox project --dry-run     # list every change, with before and after; writes nothing
wreckordbox project               # apply it, as one Run
wreckordbox project --expect PLAN # apply only if it is exactly the plan a dry run showed
```

## The policy file

`projection-policy.toml`, in the Sidecar's folder beside the Tag schema,
says for each kind of change whether it waits for you or applies at or
above a confidence:

```toml
[energy]
apply = 0.8        # Facts at least 80% sure are applied

[prep_colour]
apply = "wait"     # every Prep colour change waits for you
```

With no file, or a kind the file does not name, everything waits.

**Nothing protects this file from an agent.** It is a plain file in a
folder any program running as you can write. An agent that can edit it can
lower its own threshold and then project. Every dry run and every applied
Run names the hash of the policy it was made under (`policy_hash`, and the
Run's description), so a changed policy is visible afterwards, but it is
not prevented. Keep the file under version control if this matters to you.

## Your values are never overwritten

A projection replaces a value only when the last recorded write to that
field was a projection's own and nothing has written the track since.
Anything else is left alone and, when a Fact disagrees, listed under
"kept": a value you set by hand (even to the value it already had), one set
before wreckordbox recorded anything, one another tool or a player wrote,
and anything wreckordbox cannot account for. An empty field can be filled.

A projection is always recorded as a tool's work (`tool:projection`, or the
agent named with `--source`), in a Run of its own, never as yours. Undo in
the window takes a whole projection back in one step.

Prep colour Facts name a colour (`Green`); what you have renamed a colour
label to in rekordbox does not change which colour they mean.

## Barriers: writes wreckordbox cannot account for

If a write to the Library is begun and never confirmed (the service was
killed in the middle of it, for example during a projection), wreckordbox
cannot know what it changed. Such a write is a **barrier**: until you clear
it, Projection fills no empty field it could concern, and replaces no value
it projected before it. A write that names its tracks bars only those
tracks; one that names none, such as a crashed `wreckordbox project`, bars
every track, so nothing is filled at all.

```sh
wreckordbox projection barriers                       # which write, when, which tracks; writes nothing
wreckordbox projection acknowledge BARRIER --source dj  # clear one, as your decision
```

Clearing a barrier is yours alone: it is refused unless it names you
(`--source dj`), it is recorded in the Ledger as yours, in a Run of its
own, with the barrier as it was listed, and nothing else ever clears one (a
dry run, a listing, a projection). A write still in progress cannot be
cleared. Look at the tracks first: the barrier means some of their values
may have been written without a record.

**What a cleared barrier allows, exactly:** Projection may fill an empty
field the barrier concerned, one no recorded write touched. Nothing else
changes. A value the cleared write may have left has no record, so it is
yours and is kept; a value a projection wrote before the barrier stays
yours too, because the cleared write may have changed it afterwards.
Filling one field of a track never hands another field of it back to
Projection.

There is no way yet to give such a value back to Projection. Clearing the
field by hand does not do it: your clear is recorded as yours, so
Projection keeps the empty field as your choice and does not fill it. If a
Fact says the value should change, set it yourself.

### Values projected before this version

A projection keeps a record of what it wrote, so it can change its own
values later. Records kept by an earlier version do not say which write
they belong to, so they prove nothing. An Energy or Prep colour value an
earlier version projected therefore counts as yours: the dry run lists it
as kept with the reason `predates_confirmed_marks`, and no projection
changes it. Nothing is converted and no Fact is recorded for you.

There is no way to hand such a value back to Projection. Clearing it by
hand does not do it either: your clear is recorded as yours. If a Fact
says the value should change, set it yourself.

## What waits for you

Changes the policy holds back wait in the review queue: see
[review.md](review.md) to list them and accept or reject them.

## MyTags from the Tag schema

`wreckordbox project --mytags` projects the Tag schema (`tag-schema.toml`)
to native MyTags: each category the schema's deck view names, with its
tags, put on each track from the standing Facts of that dimension.

```sh
wreckordbox project --mytags --dry-run         # every category, tag and assignment; writes nothing
wreckordbox project --mytags --expect PLAN     # apply exactly what that dry run listed
```

The policy's own section decides whether changes apply or wait:

```toml
[mytags]
apply = 0.8      # or "wait"; no section waits
```

- **Matching.** A deck category is the Library's category of the same
  name, a deck tag the MyTag of exactly the same text in it. A category
  the deck view does not name, a dimension it leaves out, and a MyTag the
  schema does not have are never written. A term with no MyTag yet is
  listed under `definitions` and its assignments under `blocked`; applying
  is refused until the MyTag exists (make it with `wreckordbox mytags
  create`). Categories and tags are not renamed, reordered or made by a
  projection yet.
- **Parents.** With `[deck.genre] tag_parents = true`, a track whose Fact
  names a child term gets the parent's MyTag too (`implied`), with no Fact
  of its own.
- **Your MyTags are yours.** Each assignment (one track, one MyTag) is the
  projection's only if the latest recorded write to it is a projection's
  and its rows in the Library are exactly those that write left. A MyTag
  you put on or took off yourself, in the window, the CLI as `--source dj`,
  or in rekordbox, is kept. Your Library's existing MyTags have no record
  at all, so they are all yours: `wreckordbox schema convert` turns them
  into your Facts without writing the Library, and projecting straight
  after it takes nothing off and changes nothing you set, with one
  exception it lists: when most of your tracks carry a child genre with
  its parent, the converted schema says parents are shown, so a track that
  carries a child alone gets its parent added (an `implied` change). It
  waits for you unless the policy's `[mytags]` section allows it. From
  then on a projection only adds MyTags your Facts call for where none was
  ever set, and changes again only what it put on or took off itself.
- **Confirming a MyTag is a choice.** Ticking a MyTag a track already has
  (or taking off one it does not have) in the window, in Intake's tag
  panel, or with `wreckordbox mytags assign|unassign ... --source dj`, is
  recorded as yours, though nothing changes: a projection keeps it from
  then on. In a selection, only the tracks that changed are the Undo step.
- **After an Undo**, a MyTag the Undo took off stays off: the projection
  does not put it back.
- **Applying** is one Run by `tool:projection` (never yours), one entry
  in the Ledger and one step of Undo. Everything it changes is checked
  again and written in one transaction: if anything was written since the
  dry run, nothing is written.
- Held changes are listed in the report's `held` only; they do not reach
  the review queue yet, and nothing there can accept them.
