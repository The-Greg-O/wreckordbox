# Review: the Facts waiting for you

When your projection policy holds a change back (the kind waits for you,
or the Fact is below the confidence you set), the Fact waits for you in
the review queue. See [projection.md](projection.md) for the policy.

```sh
wreckordbox review waiting                       # the queue; writes nothing
wreckordbox review waiting --field energy --from agent:tagger --at-least 0.8
wreckordbox review accept 12 15 --source dj      # these Facts
wreckordbox review accept --at-least 0.8 --source dj   # every waiting Fact matching a filter
wreckordbox review reject --all --source dj      # everything waiting
```

Each waiting Fact is listed with its track, the field, the value the
Library holds now and the value proposed, its source, its confidence, and
why it waits. A filter is any of `--track` (repeatable), `--field`
(`energy` or `prep_colour`), `--from` (a source) and `--at-least` (a
confidence); every waiting Fact matching all of them is chosen. A
selection names Fact ids, a filter, or `--all`: exactly one of them.

**Accepting** is your own hand set: the values are written to the Library
through the same edit path as setting them in the window, recorded in the
Ledger as yours, in your Run, as one step of Undo for the whole selection.
They are your values from then on, and no projection replaces them. Library
Protection refuses an accept like any edit. If a chosen track was written
between reading the queue and writing (by anything), the whole accept is
refused and nothing is written; read the queue again. Undo in the window
takes an accept back, and the Facts wait again.

**Rejecting** records your decision in the Ledger, as yours, in a Run of its
own, and writes nothing to the Library. A rejected Fact is never offered
again and never projected, even if you later loosen the policy. Another
source's answer for the same field is a new suggestion and may wait in its
turn. There is no way yet to take a rejection back.

Both are yours alone: they are refused unless they name you (`--source
dj`). An agent must never accept or reject for you.
