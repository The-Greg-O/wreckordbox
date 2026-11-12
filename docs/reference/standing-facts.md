# Standing Facts

`wreckordbox facts standing <TRACK_ID> <KIND>` answers one question about
one track. Use `--json` for structured output. The service command is
`facts.standing {"id":"10003","kind":"energy"}`.

Among current Facts, the order is:

1. The DJ, regardless of confidence or rules for other sources.
2. The priority assigned to that exact source in `source-precedence.toml`.
3. Confidence, highest first.
4. The latest Fact's insertion identifier, so equal answers resolve consistently
   even if the clock changed.

A source's older, replaced Facts never stand. Measurements must match the
track's current audio; the query fingerprints that audio before choosing.
Missing or unreadable audio is an error. Judgements need no audio read.

The source rules live beside `sidecar.db`, `kinds.toml` and `tag-schema.toml`:

```toml
[sources]
"agent:trusted-tagger" = 10
"tool:meter" = 5
"web:https://example.test/releases" = -2
```

Higher integers win. Unlisted sources have priority zero; without a file all
non-DJ sources tie. The file is read on each query and can be versioned by the
DJ. Source names are exact and case-sensitive. `agent`, `tool` and `web` need
a non-empty name after the colon. A rule for the DJ, an unknown key, a malformed
source or a non-integer priority is refused with `source_precedence_invalid`,
which names the file. Unreadable rules are refused too; there is no silent
fallback to earlier rules or default priorities.

The response contains:

- `track`, `kind`, and `standing` (the Fact, or null).
- `reason`: `code`, readable `message`, and the winner's `source_priority`.
  Codes are `dj`, `source_precedence`, `confidence`, `latest_fact`,
  `only_candidate`, and `no_eligible_fact`. The reason describes the comparison
  against the next best eligible Fact.
- `superseded`: the original history plus answers currently outranked. For a
  currently outranked answer, `superseded` and `superseded_by` are computed for
  this response. Its source, value, confidence, Run and time are preserved.
  An already archived Fact retains its original replacement link.
- `stale`: Measurements whose audio differs.
- `rules_file`: the location of the versionable file, whether it exists or not.

This query creates no Sidecar or Run, changes no Fact or attachment, and sends
no event. Without an open writer it uses an immutable snapshot and refuses an
unreadable, busy or incompatible store. Audio verification affects only this
answer, including when previously stale audio returns. Re-attachment remains a
producer's operation; this query follows the existing attachment mapping.
`facts show` remains the provenance listing and does not apply source rules.

The reusable resolver is `wreckordbox_sidecar::precedence::Rules`. Callers
supply one track and kind and verify Measurement fingerprints before resolving.
Projection must also validate values against the current Tag schema.

Library hand edits made by the window, or by the CLI with `--source dj`,
record matching DJ Facts through the existing Ledger recording point. The
source is self-declared: agents and Projections must retain their own source.
An ordinary CLI edit defaults to `tool:wreckordbox` and produces no DJ Fact.

Native fields are Energy (star rating), Prep colour (the canonical colour),
title, artist, and MyTags mapped through the current Tag schema's deck labels
to stable term identifiers. Only the fields and memberships changed by the
recorded payload are considered. Renaming, moving or reordering vocabulary
never changes track Facts. Terms follow dimension order, as in `schema convert`;
a tree keeps only the most specific terms. Single-text dimensions use one term.
Clearing a scalar withdraws the DJ opinion; it creates no standing null. Clearing
a list explicitly records `[]`. Unchanged native terms may be retained only
when an existing DJ Fact attributes them to the DJ; otherwise the dimension is
skipped, rather than claiming an agent's other terms as the DJ's judgement.

Fact derivation cannot refuse an accepted Library edit or prevent its Ledger
entry. Invalid or missing schemas, unmapped tags, incompatible dimensions and
Fact insertion failures are recorded as `facts_not_recorded` (track, kind,
reason) on the entry. `runs show` displays these reasons in JSON and text. A
later repair edit remains possible. No schema is needed for rating, colour,
title or artist. Native capture never opens audio, observes attachments or
marks Measurements stale; a Judgement has no fingerprint requirement.

The Library and Sidecar have no shared transaction. The Ledger commits its
intent before the Library write. At the single recording point, it derives
Facts from the entry's before/after payload and commits the entry, successful
Facts, their links, undo effects and intent removal in one Sidecar transaction.
Fact-batch failure rolls back that batch and records the entry with reasons.
A refused or failed Library write clears its intent without an entry or Fact.
Failure of the Ledger itself still uses the existing spill/replay mechanism.
Unnamed writes settle their intent immediately. Reads return recorded Facts
and never recover, write, or refuse merely because an intent exists.

Recovery evidence is bounded to explicitly named tracks, read on the existing
writer connection before the write, and used only if the entry was interrupted.
Undo/redo names only the original edit's exact fields for this evidence.
On startup, an outstanding intent becomes an `unconfirmed` entry. Recovery
reconciles its bounded native fields with the actual Library; missing or legacy
unbounded evidence is skipped with a reason. `ledger_entry` identifies a Fact's
entry, and `reconciled: true` distinguishes recovery from a normal edit.

Undo withdraws Facts the edit produced and restores the previous Fact rows,
with their original identifiers, sources and Runs. It never asserts the previous
Library value as the DJ's. Redo records the DJ judgement again. Undoing or
redoing an agent edit does not promote it to a DJ judgement. Sidecar schema 8
keeps these effects in `dj_fact_effect`; Unwind must follow both that journal
and `dj_fact`. Linked native Facts belong to their Library entry and do not
inflate Run counts with a second edit.
