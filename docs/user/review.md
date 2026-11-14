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
wreckordbox review rejected                      # what stands rejected; writes nothing
wreckordbox review reopen --rejection ENTRY --source dj  # undo one whole reject
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
Protection refuses an accept like any edit.

An accept applies exactly what you were shown. The command reads the queue
first and sends those rows with the accept; if any of them changed before
the write (another answer replaced it, it moved to another track, it no
longer waits), or a chosen track was written by anything (even a player
or rekordbox), the whole accept is refused, naming the Facts, and nothing
is written; read the queue again. Should the write stop part-way, what was
applied is recorded as yours (one Undo step) and the answer names what was
and was not applied; what was not still waits. If wreckordbox is killed
part-way, the next start records the values written as yours. Undo in the
window takes an accept back, and the Facts wait again.

**Rejecting** records your decision in the Ledger, as yours, in a Run of its
own, and writes nothing to the Library. What you reject is the suggestion:
this value for this field of this track, from this source. It is never
offered again and never projected, even if you later loosen the policy,
and even if the same source sends the same answer again. A different value,
or the same value from another source, is a new suggestion and may wait in
its turn.

**Reopening** takes a rejection back: by the Fact you rejected, by a filter,
`--all`, or one whole earlier `reject` by its Ledger entry (`review
rejected` shows it). It is recorded as yours; the rejection stays in the
Ledger as it was. The reopened Facts wait again.

All three are yours alone: each request must name you (`--source dj`),
every time, even inside a Run of yours, or it is refused. An agent must
never accept, reject or reopen for you.
