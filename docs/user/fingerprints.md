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
