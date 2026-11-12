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

Automatic recording of Library hand edits as DJ Facts and atomic writes across
the Library and Sidecar are still outstanding parts of wbx-19. This query
resolves Facts already recorded by `facts record`, conversion or other producers;
it does not turn a Library edit into a Fact.
