# Audio fingerprints and older measurements

Wreckordbox identifies audio from its encoded stream and decoding settings,
including codec and sample format. Moving a file or editing its tags does
not change that identity. New measurements use a versioned fingerprint that
also distinguishes identical bytes interpreted as integer or floating-point
audio.

Your existing Stems and Intensity measurements keep their original fingerprint
rules. Installing this change does not mark them all stale. Looking at Facts,
previewing a Stick load, or rendering never upgrades a stored measurement.
A Stick preview and its confirmed load use the same rule for that stored
measurement; the load verifies the audio afresh.

A reused measurement keeps its old evidence version, including when Intensity
is aligned to a new beat grid or reused for another track. A new analysis
records a new version-2 Fact and leaves earlier evidence in history. To
remeasure Intensity with the new rule, run `wreckordbox intensity measure ID
--force`. Stems acquire the new version when separation actually produces a
new set, such as after the audio changes or saved Stem files are missing.
Simply running `stems separate` on a reusable set keeps its original version.

Version-1 (`a1`) evidence has two remaining limits. A format-only change,
with identical encoded bytes, rate and channel count, is not detected until
the Stems are actually made again. Reusing an Intensity measurement on another
track records the new Fact as `a1` too: copying evidence cannot prove how its
original bytes were decoded. Only a new analysis can establish `a2` evidence.

The old AIFF reader also included bytes after its sound chunk. A key edit made
by Wreckordbox can therefore make **old `a1` AIFF Stems** appear stale. Certain
external trailing tags on FLAC and M4A have similar legacy limitations. Those
old fingerprints cannot be repaired or upgraded by looking at the track.
New `a2` evidence excludes this metadata and stays current after retagging.
MP3 duration estimates are excluded too, including for files without a Xing
or Info header. The app's audio-file key writer currently supports MP3, AIFF
and FLAC; it skips WAV and M4A.
