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
