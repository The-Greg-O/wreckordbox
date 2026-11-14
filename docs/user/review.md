# Review: the Facts waiting for you

When your projection policy holds a change back (the kind waits for you,
or the Fact is below the confidence you set), the Fact waits for you in
the review queue. See [projection.md](projection.md) for the policy.

```sh
wreckordbox review waiting                       # the queue; writes nothing
wreckordbox review waiting --field energy --from agent:tagger --at-least 0.8
wreckordbox review accept 12 15 --token TOKEN --source dj      # these Facts
wreckordbox review accept --at-least 0.8 --token TOKEN --source dj   # matching a filter
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

An accept applies exactly what you were shown. `review waiting` ends with
a token naming that listing; an accept needs it (`--token`) and chooses
only within it. If anything in that listing changed since you read it
(another answer replaced a Fact, a Fact moved to another track, it no
longer waits, a new one arrived), or a chosen track was written by
anything (even a player or rekordbox, up to the moment of the write), the
whole accept is refused, showing what the Facts are now, and nothing is
written: list again and decide again. Should the write stop part-way, what was
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
its turn. When the music of a track is added again and its Facts follow
it to the new track, your rejections follow them too.

**Reopening** takes a rejection back: by the Fact you rejected, by a filter,
`--all`, or one whole earlier `reject` by its Ledger entry (`review
rejected` shows it). It is recorded as yours; the rejection stays in the
Ledger as it was. The reopened Facts wait again.

All three are yours alone: each request must name you (`--source dj`),
every time, even inside a Run of yours, or it is refused. An agent must
never accept, reject or reopen for you.
