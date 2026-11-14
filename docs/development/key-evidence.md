# Evidence carried by the key writer (wbx-dua)

A version-1 AIFF fingerprint includes bytes from the trailing ID3 chunk. A
verified key-tag rewrite can therefore change a1 while leaving the audio
identical. The key writer now carries the track's existing Measurements,
including Stems, during that publication. This is the DJ's option 2 decision;
it is neither new analysis nor an upgrade to version 2.

## Authority and timing

`rbl-db::key_tag::prepared::Prepared` exists only after the existing key writer
has compared the original and rewritten audio byte for byte. `inspect_verified`
binds an observation to that exact staged copy and checks both metadata stamps
before and after inspection. A different plan, unpublished plan, modified copy,
or rolled-back publication cannot supply `published_verification`.

The service inspects only the selected tracks' live legacy Measurements and
fingerprints the original and staged copies outside the Library edit gate.
When both have a2 identities, those must also agree. No observation is recorded at this stage.
A missing service recorder leaves evidence alone. A failed inspection refuses
preparation, before native/file changes. No public service command accepts a
claimed verification, an old fingerprint, or a replacement fingerprint.

After publication, and only for files actually retained by the edit, the
captured evidence is recorded. Undo/Redo performs fresh preparation and the
same verification and recording. Failed file/native history steps do not
reattest evidence; it may remain stale. The ordinary edit path, Protection,
publication authorization and rollback behavior are retained.

## Exactly what is stored

For each captured Measurement still live on the same track and still carrying
the verified before fingerprint, a new Fact copies its kind, value, producer
source and confidence. Its fingerprint is the **after legacy digest**, `a1:…`
(or the existing `f1:…` fallback), never a2. The original Fact's fingerprint,
value, source and original Run remain intact; normal supersession links it to
the copied Fact. Stem files, their hashes and `produced_by` are copied as
references, not regenerated. Intensity values retain their original method.

A format-1 `key_evidence_carry` Ledger entry records the copying act as
`tool:wreckordbox-key-tag`, even when a DJ requested the native key edit. Its
payload records `verification: key_writer_byte_for_byte`, `evidence_version: 1`,
before/after legacy fingerprints and every track, kind, old/new Fact id,
original Measurement source and original Measurement Run. The source on a
copied Fact describes the original Measurement's provenance; the dedicated
Ledger entry identifies who carried it. This does not claim that its producer
ran again. Copied Facts belong to the carry operation's Run.

The window's native key edit remains the DJ's act. Its carry entries use a
separate key-writer Run, coalesced across nearby publications. Within Analysis
or another existing bulk operation they join that one Run, while their entry
source remains the key writer's. No Fact or entry changes when legacy digests
are equal (ordinary MP3/FLAC). No global fingerprint equivalence is introduced:
only the captured Facts on the named tracks can carry.

## Failure, reads and bounds

At most 128 Facts are copied per transaction, with their supersessions and
explicit Ledger entry. A new writer Run is created in that same transaction.
A failure leaves no unrecorded copies or empty new Run. Earlier successful
batches remain recorded; later batches remain stale, and the command reports
that evidence was not fully carried. An accepted key edit/Undo/Redo is recorded
and kept even when evidence recording fails. No retry/recovery is performed
by a read or at startup.

Readers use the existing version-aware fingerprint comparison. They still
verify current audio and the saved Stem files; an audio change makes carried
evidence stale. External tag edits receive no carry authority. Reads neither
publish nor upgrade evidence. There is no schema migration or change to the
fingerprint algorithm. When legacy candidates exist, preparation adds two
fingerprint reads; neither audio read occurs under the Library gate or a plan
ownership mutex.
