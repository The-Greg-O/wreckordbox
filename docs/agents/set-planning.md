# Set planning: a guide for agents

Read this before you write a Recipe or build a Set plan. It gives the
musical and arithmetic basis of harmonic mixing, tempo and energy flow, and
how each maps onto the Recipe that `wreckordbox set-plan build` reads. Terms
are as in the project's glossary: Recipe, Set plan, Situation, Intensity,
Energy. Print this guide with `wreckordbox set-plan guide` (`--json` gives
it in sections), or call the MCP tool `read_set_planning_guide`; neither
needs the service or reads the Library.

Every table and number below is recomputed by a test
(`crates/wreckordbox-setplan/tests/guide.rs`) from pitch-class sets, equal
temperament and this app's own key arithmetic and scoring, so the document
cannot drift from the arithmetic. Each statement is marked by kind:
**derived** (acoustics, equal temperament, set arithmetic: checkable),
**convention** (DJ practice or one vendor's guidance, attributed by name),
or **unverified** (not checked). Everything here is a default the DJ
overrides.

## 1. Keys

### Equal temperament and the circle of fifths (derived)

Pitch classes are numbered C=0, C#/Db=1, D=2, Eb=3, E=4, F=5, F#/Gb=6,
G=7, Ab=8, A=9, Bb=10, B=11, with arithmetic mod 12. In twelve-tone equal
temperament a semitone is the frequency ratio 2^(1/12) (about 1.059463),
100 cents; s semitones are 2^(s/12).

The pure fifth is 3:2 (1200 x log2(1.5) = 701.96 cents); the tempered fifth
of 7 semitones is 2^(7/12) = 1.498307, 700 cents, 1.96 cents flat. Tempered
thirds are much further from pure: the major third is 13.69 cents sharp of
5:4 and the minor third 15.64 cents flat of 6:5.

Because 7 and 12 are coprime, repeatedly adding 7 (a fifth) visits all
twelve pitch classes before returning: the circle of fifths. And
7 x 7 = 49 = 1 mod 12, so 7 is its own inverse mod 12.

A major scale on tonic p is p + {0, 2, 4, 5, 7, 9, 11}: seven consecutive
places on the circle of fifths, p + 7j for j = -1..5. Moving the tonic a
fifth up shifts that window by one place: six tones stay, the fourth
degree rises a semitone (C major to G major: F becomes F#). So keys a
fifth apart share six of seven tones. In general two scales of the same
mode d places apart on the circle share max(2, 7 - d) tones (two 7-tone
sets out of 12 always share at least 7 + 7 - 12 = 2):

<!-- table: shared-by-distance -->
| Places apart on the circle (d) | Shared scale tones |
|---|---|
| 0 | 7 |
| 1 | 6 |
| 2 | 5 |
| 3 | 4 |
| 4 | 3 |
| 5 | 2 |
| 6 | 2 |

### Relative and parallel keys (derived)

- The natural minor scale on p is p + {0, 2, 3, 5, 7, 8, 10}: the same seven
  tones as the major scale on p + 3, its **relative major** (A minor and C
  major). They share 7 of 7 scale tones and 2 of 3 tonic-triad tones
  (A minor {A, C, E}, C major {C, E, G}). A minor track that raises its
  seventh (harmonic minor, G# in A minor) shares 6 of 7 with the relative
  major and clashes with its fifth (G).
- The **parallel** major and minor share the tonic: natural scales share 4
  of 7 tones ({0, 2, 5, 7} above the tonic; 5 of 7 against harmonic minor,
  6 of 7 against ascending melodic minor) and 2 of 3 tonic-triad tones
  (root and fifth). The parallel major's scale lies 3 places clockwise of
  the minor's on the circle.

Standard references for these facts: Kostka and Payne, *Tonal Harmony*;
Aldwell and Schachter, *Harmony and Voice Leading* (print, not opened for
this guide).

### The Camelot wheel and Open Key

The Camelot wheel numbers the circle of fifths like a clock: 1 to 12, one
fifth up per step; `A` is the minor ring, `B` the major ring, `nA` the
relative minor of `nB`, anchored at 8A = A minor and 8B = C major
(**convention**: devised by Mark Davis and popularised by Mixed In Key; see
References). Open Key (beaTunes, used by Traktor) is the same clock turned
so that 1m = A minor and 1d = C major. With p the tonic's pitch class:

- Camelot k to tonic: minor p = (7k + 1) mod 12; major p = (7k + 4) mod 12.
- Tonic to Camelot: minor k = ((7p + 4) mod 12) + 1; major k = ((7p + 7) mod 12) + 1.
- Camelot to Open Key: o = ((k + 4) mod 12) + 1 (A to m, B to d); back:
  k = ((o + 6) mod 12) + 1.
- Transposing a key up s semitones moves it +7s mod 12 on the wheel, same
  ring; a wheel step of +k transposes by 7k mod 12 semitones.

<!-- table: camelot -->
| Camelot | Minor key | Tonic | rekordbox | Open Key | Camelot | Major key | Tonic | rekordbox | Open Key |
|---|---|---|---|---|---|---|---|---|---|
| 1A | G#m / Abm | 8 | Abm | 6m | 1B | B / Cb | 11 | B | 6d |
| 2A | D#m / Ebm | 3 | Ebm | 7m | 2B | F# / Gb | 6 | F# | 7d |
| 3A | Bbm / A#m | 10 | Bbm | 8m | 3B | Db / C# | 1 | Db | 8d |
| 4A | Fm | 5 | Fm | 9m | 4B | Ab / G# | 8 | Ab | 9d |
| 5A | Cm | 0 | Cm | 10m | 5B | Eb / D# | 3 | Eb | 10d |
| 6A | Gm | 7 | Gm | 11m | 6B | Bb / A# | 10 | Bb | 11d |
| 7A | Dm | 2 | Dm | 12m | 7B | F | 5 | F | 12d |
| 8A | Am | 9 | Am | 1m | 8B | C | 0 | C | 1d |
| 9A | Em | 4 | Em | 2m | 9B | G | 7 | G | 2d |
| 10A | Bm | 11 | Bm | 3m | 10B | D | 2 | D | 3d |
| 11A | F#m / Gbm | 6 | F#m | 4m | 11B | A | 9 | A | 4d |
| 12A | C#m / Dbm | 1 | Dbm | 5m | 12B | E | 4 | E | 5d |

The rekordbox column is the Classic spelling this app shows and stores
(`rbl-index` `key.rs`); it was not checked against rekordbox itself. The
app reads all the spellings in the table, Camelot codes and Open Key.

### Moves between keys

For keys (k1, ring1) to (k2, ring2): the clockwise step is
(k2 - k1) mod 12, the tonic shift is (p2 - p1) mod 12 semitones up. "App
moves" is what this app counts (`Camelot::moves`: the shorter way round
the wheel, plus 1 for a change of ring) and "Key score" its contribution at
weight 1, max(0, 1 - moves / 4). Shared tones and triad tones are derived;
names and uses are **convention** (Mixed In Key unless marked; the
consonant/clashing diagonal split is Mixed In Key's *Beyond Beatmatching*,
ch. 8, as reported by the research review, and agrees with the triad
count).

<!-- table: moves -->
| Move | From | To | Arithmetic | Tonic up | Shared tones | Triad tones | App moves | Key score | Use |
|---|---|---|---|---|---|---|---|---|---|
| Same key | 8A | 8A | k | 0 | 7 | 3 | 0 | 1 | long blends |
| +1, fifth up | 8A | 9A | k + 1 | 7 | 6 | 1 | 1 | 0.75 | long blends |
| -1, fifth down | 8A | 7A | k - 1 | 5 | 6 | 1 | 1 | 0.75 | long blends |
| Relative major | 8A | 8B | k, ring | 3 | 7 | 2 | 1 | 0.75 | long blends; brighter |
| Relative minor | 8B | 8A | k, ring | 9 | 7 | 2 | 1 | 0.75 | long blends; darker |
| Consonant diagonal | 8A | 7B | k - 1, ring | 8 | 6 | 2 | 2 | 0.5 | medium blends |
| Consonant diagonal | 8B | 9A | k + 1, ring | 4 | 6 | 2 | 2 | 0.5 | medium blends |
| Clashing diagonal | 8A | 9B | k + 1, ring | 10 | 6 | 0 | 2 | 0.5 | drums only, or a cut |
| Clashing diagonal | 8B | 7A | k - 1, ring | 2 | 6 | 0 | 2 | 0.5 | drums only, or a cut |
| +2, "Energy Boost" | 8A | 10A | k + 2 | 2 | 5 | 0 | 2 | 0.5 | short blends, drops |
| -2, whole tone down | 8A | 6A | k - 2 | 10 | 5 | 0 | 2 | 0.5 | short blends; a release |
| Parallel major | 8A | 11B | k + 3, ring | 0 | 4 | 2 | 4 | 0 | breakdown to drop |
| Parallel minor | 8B | 5A | k - 3, ring | 0 | 4 | 2 | 4 | 0 | breakdown to drop |
| Tritone ring flip | 8A | 5B | k - 3, ring | 6 | 4 | 0 | 4 | 0 | avoid |
| -3, "Pay Attention" | 8A | 5A | k - 3 | 3 | 4 | 1 | 3 | 0.25 | short blends, cuts |
| +3 | 8A | 11A | k + 3 | 9 | 4 | 1 | 3 | 0.25 | short blends, cuts |
| +4, major third | 8A | 12A | k + 4 | 4 | 3 | 1 | 4 | 0 | cuts |
| -4 | 8A | 4A | k - 4 | 8 | 3 | 1 | 4 | 0 | cuts |
| +7, semitone up ("Jaws Mix") | 8A | 3A | k + 7 | 1 | 2 | 0 | 5 | 0 | cuts only |
| -7, semitone down | 8A | 1A | k - 7 | 11 | 2 | 0 | 5 | 0 | cuts only |
| +6, tritone | 8A | 2A | k + 6 | 6 | 2 | 0 | 6 | 0 | hard cuts only |

Notes:

- Same key, +1, -1 and A/B with the same number are Mixed In Key's basic
  compatible moves. +2 is two fifths up, a whole tone (A minor to B minor).
  +7 is +7 x 7 = 49 = 1 semitone; Mixed In Key also writes it as -5 and
  calls it the Jaws Mix (**convention**).
- The two diagonals share six scale tones each. The consonant one (8A to
  7B, 8B to 9A) keeps two tonic-triad tones (a third-related tonic); the
  clashing one (8A to 9B, 8B to 7A) keeps none, and a harmonic-minor
  leading tone clashes by a semitone with the major key's tonic.
- Parallel major and minor share the bass root and fifth; do not layer
  the two different thirds. The other A/B flip at three places (8A to 5B)
  puts the roots a tritone apart.
- Shared tones are a proxy. Unpitched intros and outros mix in any key; a
  low cut on one deck at the phrase boundary removes most low-end
  clashing; two lead vocals clash even in one key (**convention**).

## 2. Tempo

### Percent, ratio and BPM (derived)

A change from x to y BPM is (y - x) / x x 100 percent of the outgoing
tempo; one BPM is 100 / x percent (0.83% at 120, 0.78% at 128). Perception
and time-stretching both follow the ratio y / x, so a fixed BPM step is a
different change at different tempos:

<!-- table: bpm-steps -->
| From | To | Change (%) | Semitones without key lock | Cents |
|---|---|---|---|---|
| 70 | 74 | 5.714 | 0.962 | 96.2 |
| 100 | 104 | 4 | 0.679 | 67.9 |
| 125 | 129 | 3.2 | 0.5453 | 54.5 |
| 140 | 144 | 2.857 | 0.4877 | 48.8 |
| 174 | 178 | 2.299 | 0.3935 | 39.3 |

Percent of the outgoing tempo is not symmetric: 120 to 125 is +4.167%,
125 to 120 is -4.000%, though both are the same stretch (|ln(125/120)| =
0.0408). The fader setting the incoming track needs to meet the outgoing
one is x / y - 1: from 120 to 126 BPM the change is +5.00% of the outgoing
tempo, but the incoming track is played at -4.76%.

### Pitch without key lock (derived)

Without key lock, a player changes tempo by playing faster, so every
frequency scales by 1 + p / 100 and pitch moves by
semitones = 12 x log2(1 + p / 100); inversely p = 100 x (2^(s/12) - 1).

<!-- table: tempo-pitch -->
| Tempo change (%) | Up: semitones | Up: cents | Down: semitones | Down: cents |
|---|---|---|---|---|
| 1 | 0.1723 | 17.2 | -0.174 | -17.4 |
| 2 | 0.3428 | 34.3 | -0.3498 | -35 |
| 3 | 0.5117 | 51.2 | -0.5273 | -52.7 |
| 4 | 0.679 | 67.9 | -0.7067 | -70.7 |
| 5 | 0.8447 | 84.5 | -0.888 | -88.8 |
| 6 | 1.0088 | 100.9 | -1.0712 | -107.1 |
| 7 | 1.1713 | 117.1 | -1.2564 | -125.6 |
| 8 | 1.3324 | 133.2 | -1.4435 | -144.4 |

One semitone up is +5.9463% (2^(1/12) - 1); one semitone down is -5.6126%
(2^(-1/12) - 1). Half a semitone (50 cents) is +2.9302% or -2.8468%; two
semitones are +12.2462% or -10.9101%.

**The effective key after a tempo change.** If track A (at BPM a) and
track B (at BPM b) are beatmatched without key lock, B sounds
12 x log2(a / b) semitones higher relative to A, whichever deck moves. Write
that as n + e, n the nearest whole number of semitones, e the rest:

- n moves B's effective key 7n mod 12 places on the wheel, same ring. A
  track at 120 BPM in 1A played at 127.1 BPM (+5.92%) rises 0.9952
  semitones and sounds in 8A, 0.48 cents flat: a match with an 8A track at
  127.1. The same tempo change on an 8A track makes it 3A, a semitone clash.
- e is a detuning between keys. At |e| from 0.25 to 0.5 semitones (tempo
  changes from about 1.45% to 4.43%) neither key fits: two 8A tracks at 125
  and 129 BPM are 54.5 cents apart, and their bass at 110 Hz beats at 3.5 Hz,
  at 440 Hz at 14.1 Hz.

As a working rule, a tempo change of up to about 1.5% (26 cents) leaves the
labelled key usable without key lock (**convention**; the threshold is a
judgement, not a measurement).

**Key lock.** Pioneer DJ's Master Tempo, Serato's and Engine DJ's Key
Lock, Traktor's Key Lock keep pitch while changing tempo by
time-stretching. Its artefacts (softened or doubled transients, a watery or
metallic top, smeared sub-bass) grow with the stretch (derived in kind; how
much at a given setting depends on the algorithm and is **unverified** for
any player). Ranges DJs describe (**convention**, as gathered by the
research review): up to 1% many play without key lock (17 cents); up to 3
to 4% key lock is transparent for long blends; 4 to 6% works for shorter
blends; beyond 8 to 10% artefacts are plain. Pioneer players offer fader
ranges of 6, 10 and 16 percent and a wide range (**unverified** per model).

### Half and double time, and drift (derived, then convention)

A track at 70 BPM sits on a 140 BPM grid at double time with no stretch
and no pitch change; so does 87 against 174. The tempo distance folded by
octaves is min over k in {-1, 0, 1} of |2^k y - x| / x. Bridges such as 70 to
140 (halftime to dubstep) and 85 to 170 (hip-hop to drum and bass) are
common (**convention**, Mixed In Key's *Power Block Mixing*).

Per transition, 1.5 to 3% is commonly preferred for long blends and 4 to
6% is a practical ceiling with key lock; over a one-hour club slot a drift
of some 3 to 8 BPM upwards is typical (**convention**, from the research
review's sources; **unverified** here). A per-transition limit does not
bound the drift: m percent per step over n tracks allows up to
(1 + m / 100)^(n - 1) - 1, which is +80.1% for 4% over 16 tracks and
+16.1% for 1%.

## 3. Energy and Intensity

- **Energy** is the DJ's own 1 to 5 judgement, stored as the track's star
  rating. It is not measured and the Recipe has no field for it; use it to
  choose the pool (`set-plan build ... --energy 3..5`). Mixed In Key's own
  Energy Level runs 1 to 10 and is a different, measured scale
  (**convention**).
- **Intensity** is this app's measurement (0 to 1) of how hard a track hits
  and how that changes over its length (`wreckordbox intensity measure`):
  a weighted sum of loudness (0.40), onset density (0.20), low end (0.15),
  onset strength (0.15) and high end (0.10), each scaled between a floor and
  a ceiling. The Recipe works on the track's summary Intensity, the mean of
  its curve. It is not yet calibrated against Energy.

Why summary Intensity can disagree with the DJ's Energy:

- Loudness follows mastering: a modern brickwalled master reads louder
  than a dynamic older one, though the DJ gains them to the same level.
- Onset density's ceiling is 8 a second; steady sixteenth notes give
  4 x BPM / 60 onsets a second (8.00 at 120, 8.53 at 128, 9.33 at 140), so
  if every sixteenth is detected it is at or over the ceiling at club tempos.
- A whole-track mean counts a long breakdown: a peak track with a
  two-minute breakdown can read lower than a flat warm-up loop.
- Groove, vocals, familiarity and genre are not measured.

So treat Intensity as relative within one Library; read a pool's spread
(`intensity show`) before choosing shape values, and pull the shape into
the range the pool covers.

### Set shapes as Intensity curves

The Recipe's shape is points `{ at, intensity }`, `at` from 0 (first
track) to 1 (last), joined by straight lines and flat outside the first
and last points. Position i (from 0) of n tracks is at i / (n - 1). These
arcs are **convention** (gathered by the research review from Brewster and
Broughton, *How to DJ Right*; Mixed In Key; Golden and Etro, *The Time Line
of a Dance Floor*):

| Shape | Use | Points (at, intensity) |
|---|---|---|
| Warm-up | opening 60 to 90 minutes | (0, 0.28), (0.45, 0.42), (0.8, 0.56), (1, 0.62) |
| Build to peak | a 60 to 90 minute showcase | (0, 0.45), (0.35, 0.65), (0.5, 0.58), (0.82, 0.92), (1, 0.85) |
| Peak-time hour | main slot | (0, 0.72), (0.3, 0.88), (0.5, 0.7), (0.8, 0.92), (1, 0.82) |
| Closing | after the peak | (0, 0.78), (0.25, 0.82), (0.65, 0.58), (0.9, 0.42), (1, 0.35) |
| Waves | a long resident set | (0, 0.35), (0.22, 0.7), (0.32, 0.5), (0.6, 0.85), (0.7, 0.62), (0.9, 0.92), (1, 0.55) |

A warm-up stays below the next DJ; a peak hour breathes once before its
climax; waves give dancers rests so each peak feels higher.

### Step size between neighbours (derived)

On a segment from (at1, i1) to (at2, i2), neighbouring targets differ by
|i2 - i1| / (at2 - at1) / (n - 1). If `intensity.max_step` is below the
steepest segment's step, the shape cannot be followed and the plan lags
behind it; leave slack for a pool that does not have the exact values
(the research review suggests 0.05 to 0.10, **convention**). Steps between
neighbours that DJs describe (**convention**): up to about 0.06 to 0.12
feels seamless; +0.10 to +0.18 is a deliberate lift; -0.12 to -0.22 a
breather after a climax; beyond 0.25 to 0.30 a change of regime for a cut
or a breakdown.

### Key moves and perceived energy (convention)

Raising the tonal centre (+2, +7) is heard as a lift and lowering it (-2,
-7) as a release; minor to major brightens, major to minor darkens. Mixed
In Key pairs energy boosts with rising Energy and suggests one every 20 to
30 minutes, and cautions that a key change alone does not guarantee a rise
in energy (as reported by the research review). This app scores key moves
by distance only, without direction (section 5).

## 4. How this app sequences: the Recipe and the scores

### The Recipe

A Recipe is a TOML file, `version = 1`. A section left out is a criterion
not used: it neither scores nor limits. Unknown keys are refused. Check one
with `wreckordbox set-plan check RECIPE`: every mistake is listed with its
line.

| Field | Meaning | Hard limit? |
|---|---|---|
| `version` | 1 | |
| `name` | for the DJ | |
| `tracks` | how many tracks the plan holds, 2 to 200 | yes |
| `intensity.shape` | the curve of section 3 | |
| `intensity.max_step` | most Intensity change between neighbours, above 0, at most 1 | yes |
| `intensity.missing` | `"leave_out"` (default) or `"allow"` a track with no Intensity; `"allow"` cannot go with `max_step` | |
| `tempo.max_change` | most tempo change in percent of the outgoing tempo, above 0 | yes |
| `key.max_moves` | most app moves on the wheel, 0 to 7 | yes |
| `situations.place` | `{ situation, from, to, require }`: where a Situation belongs, as fractions; `require = true` makes every track there carry it | `require` only |
| `recency.half_life_days`, `recency.as_of` | favour recent additions; `as_of` YYYY-MM-DD, default the newest addition in the pool | |
| `genre.mix` | `"vary"` (neighbours of different genres) or `"stay"` | |
| `<section>.weight` | how much the criterion counts, 0 or more, default 1 | |

Situation names must be the Tag schema's `situation` terms when the
Library has a schema; `set-plan check` lists them when one is wrong. A
required place makes the Recipe unsatisfiable when no track of the pool
carries that Situation.

What `key.max_moves` admits, by the moves table: 0 = same key only; 1 = the
basic moves (+1, -1, relative), all safe for long blends; 2 adds both
diagonals (consonant and clashing) and +2/-2; 3 adds +3/-3; the parallel
keys need 4 and the semitone moves 5, which admit nearly everything.

### The scores

Each transition is scored on every criterion the Recipe uses, for the
incoming track. Contributions are rounded to 4 places; a transition's score
is their sum, and the plan's score is the opening (the first track's
position criteria) plus every transition.

| Criterion | Contribution (w = the section's weight) |
|---|---|
| `intensity` | w x (1 - abs(Intensity - target)) |
| `key` | w x max(0, 1 - moves / 4) |
| `tempo` | w x max(0, 1 - abs(change %) / S), S = `max_change`, or 8 without one |
| `situations` | +w if the track carries a Situation placed here, -w if it carries one placed elsewhere, else 0 |
| `recency` | w x 0.5 ^ (age in days / half_life_days) |
| `genre` | w if the mix is as wanted, else 0 |

<!-- table: key-score -->
| App moves | Key score at weight 1 |
|---|---|
| 0 | 1 |
| 1 | 0.75 |
| 2 | 0.5 |
| 3 | 0.25 |
| 4 | 0 |

Exchange rates at weight 1: one key move costs 0.25, the same as an
Intensity 0.25 off its target or a tempo change of a quarter of S (1% at
S = 4). The sum is compensatory: a transition scoring 1, 1 and 0 (a perfect
Intensity and tempo with a tritone) totals 2.0 and beats one scoring 0.65
on all three (1.95). Only the hard limits stop a bad move; set them for
what must never happen, and use weights for preferences. The research
review suggests `intensity.weight` about 1.3 to 1.6 times `key.weight`, so a
same-key run does not override the curve, and modest recency and genre
weights (0.3 to 0.6) (**convention**, a starting point to adjust).

Hard limits are never broken in a returned plan. The search is a beam of
48 partial orders, so under very tight limits it can report a Recipe
unsatisfiable when a plan exists; loosen the limit it names. Missing data
is named: a track without Intensity (unless `missing = "allow"`), without
tempo under a tempo limit or without key under a key limit is listed in
`left_out` with what it lacks; a criterion scored without data gives 0 and
says why.

### Reading a transition

`set-plan show ID --json` gives `plan.transitions[]` with `score` and
`criteria[] { criterion, score, reason }`. Worked example, weights 1,
`tempo.max_change = 4`, from a track at 124 BPM in 8A to one at 126 BPM in
9A with Intensity 0.62 where the shape wants 0.60:

<!-- table: worked-transition -->
| Criterion | Arithmetic | Score |
|---|---|---|
| intensity | 1 - abs(0.62 - 0.60) | 0.98 |
| key | 1 - 1 / 4 | 0.75 |
| tempo | 1 - (2 / 124 x 100) / 4 = 1 - 1.6129 / 4 | 0.5968 |
| total | | 2.3268 |

The reasons read "Intensity 0.62 where the shape wants 0.60, a step of
+0.02 from 0.60", "8A to 9A, 1 move" and "124.00 to 126.00 BPM, +1.6%". A
low criterion in a good plan is the cost of keeping another; an
unsatisfiable Recipe names the criterion in the way, the limits that alone
block it, and the position where it stopped.

### Example Recipes

Save one to a file, check it, then build from a pool:

    wreckordbox set-plan check warm-up.toml
    wreckordbox set-plan build warm-up.toml --playlist "Warm-up pool" --source agent:NAME
    wreckordbox set-plan show ID --json

The Situation names are examples: use the Tag schema's terms, or leave the
section out. Adapted from the research review's examples.

A warm-up hour: 12 tracks at about 5 minutes each, a steady ramp, tight
tempo, basic key moves only, one genre, an opener first.

```toml
version = 1
name = "Warm-up hour"
tracks = 12

[intensity]
shape = [
  { at = 0.0, intensity = 0.28 },
  { at = 0.45, intensity = 0.42 },
  { at = 0.8, intensity = 0.56 },
  { at = 1.0, intensity = 0.62 },
]
max_step = 0.12
weight = 1.4

[tempo]
max_change = 2.5
weight = 1.2

[key]
max_moves = 1
weight = 1.0

[situations]
place = [{ situation = "opener", from = 0.0, to = 0.0, require = true }]
weight = 0.8

[recency]
half_life_days = 120
weight = 0.4

[genre]
mix = "stay"
weight = 0.6
```

A peak-time hour: 15 tracks at about 4 minutes, high with one breather
before the climax, +2 and diagonals allowed, fresher tracks favoured.

```toml
version = 1
name = "Peak-time hour"
tracks = 15

[intensity]
shape = [
  { at = 0.0, intensity = 0.72 },
  { at = 0.3, intensity = 0.88 },
  { at = 0.5, intensity = 0.7 },
  { at = 0.8, intensity = 0.92 },
  { at = 1.0, intensity = 0.82 },
]
max_step = 0.22
weight = 1.5

[tempo]
max_change = 3.5
weight = 1.0

[key]
max_moves = 2
weight = 1.0

[situations]
place = [{ situation = "peak", from = 0.2, to = 0.86 }]
weight = 1.0

[recency]
half_life_days = 60
weight = 0.6

[genre]
mix = "stay"
weight = 0.5
```

A closing set: 13 tracks at about 5.5 minutes, a short hold and then a
landing, room for a slow tempo wind-down, a closer last.

```toml
version = 1
name = "Closing set"
tracks = 13

[intensity]
shape = [
  { at = 0.0, intensity = 0.78 },
  { at = 0.25, intensity = 0.82 },
  { at = 0.65, intensity = 0.58 },
  { at = 0.9, intensity = 0.42 },
  { at = 1.0, intensity = 0.35 },
]
max_step = 0.18
weight = 1.4

[tempo]
max_change = 4.0
weight = 0.9

[key]
max_moves = 2
weight = 1.1

[situations]
place = [
  { situation = "afterhours", from = 0.3, to = 0.92 },
  { situation = "closer", from = 1.0, to = 1.0, require = true },
]
weight = 1.2

[recency]
half_life_days = 180
weight = 0.3

[genre]
mix = "vary"
weight = 0.5
```

Each example's steepest shape segment against its `max_step` (slope in
Intensity per unit of `at`; step = slope / (tracks - 1)):

<!-- table: steepest-step -->
| Recipe | Tracks | Steepest slope | Step per track | max_step |
|---|---|---|---|---|
| Warm-up hour | 12 | 0.4 | 0.0364 | 0.12 |
| Peak-time hour | 15 | 0.9 | 0.0643 | 0.22 |
| Closing set | 13 | 0.7 | 0.0583 | 0.18 |

The warm-up's steepest segment is 0.45 to 0.8, the peak hour's the
breather from 0.3 to 0.5, the closing set's the last, from 0.9 to 1.

## 5. Known limits of the current sequencer

Checked against `crates/wreckordbox-setplan` (`camelot.rs`, `sequence.rs`)
and the service's feed. Each is a gap between the theory above and what
the sequencer computes; work round it in the Recipe until the DJ decides.

1. **It assumes key lock.** Key moves are counted on the stored keys and
   tempo separately; a tempo change never shifts the key. Without key lock
   a 3 to 4% change detunes a track by 51 to 71 cents, and about 5.95%
   moves it a whole semitone (7 places), while the key still scores as
   before. Workaround: if the DJ plays without key lock, set
   `tempo.max_change = 1.5` or less; with key lock, keep it within 4 to 6%
   for the artefacts' sake.
2. **The key distance has no direction and no triad.** `moves` is the
   shorter way round plus 1 for a ring change, so the consonant and the
   clashing diagonal both cost 2, as do +2 and -2; the parallel keys and
   the tritone ring flip both cost 4 and score 0, like a missing key.
   Workaround: `key.max_moves = 1` admits only moves that are safe for long
   blends. With `max_moves = 2`, read each transition's key reason in the
   built plan and check the 2-move ones against the moves table (a
   clashing diagonal is 8A to 9B or 8B to 7A in shape: k + 1 from A to B,
   k - 1 from B to A); "boost" is +2 clockwise on the same ring only.
3. **Tempo: no half or double time, no drift bound, and a one-sided
   percent.** 70 to 140 BPM is a 100% change. A per-step limit allows
   compound drift ((1 + m / 100)^(n - 1) - 1). The limit is a percent of
   the outgoing tempo, so 120 to 125 (+4.167%) breaks `max_change = 4`
   while 125 to 120 (-4.000%) keeps it. Workaround: keep half-time and
   double-time tracks in separate pools (or leave `[tempo]` out and check
   by hand); bound drift by the pool, choosing tracks from a BPM range
   (`tracks query --bpm 122..128`, then pass their ids); set `max_change`
   a little under the stretch you accept upwards.
4. **Intensity is scored against the curve only.** The step from the
   previous track appears in the reason but not in the score, so
   0.5, 0.7, 0.5 around a flat target of 0.6 scores 0.9 each time, the same
   as a steady 0.5, 0.5, 0.5. The value is the whole-track mean
   (`summary.intensity`), uncalibrated; a track without it is left out under
   `max_step` (there is no fallback to Energy); mix-in and mix-out points
   are not in the plan. Workaround: set `max_step` just above the shape's
   steepest step plus slack, as in section 4; choose shape values inside
   the pool's measured range; measure the pool first.
5. **Same key wins ties, and missing data scores like the worst case.**
   Moves 0 scores 1 and every other move less, so with other criteria
   equal the beam prefers staying in one key. Without hard limits a track
   with no key or no tempo scores 0, like a tritone or an 8% jump. A track
   with no Situation recorded scores 0, more than one known to belong
   elsewhere (-w): `situations` swings over 2w, the others over w.
   Workaround: lower `key.weight` against `intensity.weight` (section 4) or
   vary the genre to break same-key runs; set `max_moves` and `max_change`
   so tracks lacking key or tempo are left out and named rather than
   scored 0; record Situations for the pool, or keep their weight low.
6. **The design mock labels 8A to 6A "boost".** In
   `docs/design/screens/mock.html` that is -2, a whole tone down; only +2
   on the same ring is Mixed In Key's Energy Boost. Since `moves` has no
   direction, a view must compute (k2 - k1) mod 12 = 2 with the same ring
   itself.

## 6. Limits and honesty

- **Derived**: the semitone ratio and cents, shared scale and triad
  tones, the Camelot and Open Key arithmetic, tempo to pitch, the drift
  bound, every score in section 4.
- **Convention**: which moves are compatible, what they feel like, their
  names (Mixed In Key's harmonic mixing guide, Camelot wheel and
  *Beyond Beatmatching*), set shapes, step sizes, typical tempo changes and
  key-lock ranges. Defaults, not rules.
- **Key detection is often wrong.** Detectors force every track into one
  of 24 major or minor templates. Modal club music (Phrygian, Dorian,
  Mixolydian), root-and-fifth basslines with no third, atonal drum tools,
  pitched kick sweeps, tracks that change key, loops that sit between a
  key and its relative, and recordings tuned off A = 440 Hz all mislead
  them. Published accuracies on dance music are well below certainty: the
  research review reports exact 24-key accuracies of about 67 to 72% for
  rekordbox and Mixed In Key on the GiantSteps Key set (Knees, Faraldo et
  al., ISMIR 2015) and 41 to 86% across programs in a 66-track DJ
  TechTools test (2015). At 70% per track both keys of a pair are right
  0.70 x 0.70 = 49% of the time. A fifth error turns a planned +1 into a
  real +2; a parallel error makes this app count 4 moves against the
  true key. The DJ's ear and the DJ's corrections win.
- **Intensity** is not yet calibrated against Energy.
- These are defaults. The DJ overrides any of them.

## 7. References

Two kinds. Pages marked **opened** were fetched by the independent
research review (set-planning research, 2026-11-15), which used web
search; the URLs and figures are as it recorded them. This guide's own
author had no network: the arithmetic was derived and is recomputed by the
test, and nothing marked opened was re-opened here. Items marked
**not opened** are cited from established knowledge only.

Vendor guides and notations:

1. Mixed In Key, *Harmonic Mixing Guide* (updated September 2024): the
   1A to 12B wheel, the basic moves, Energy Levels. Opened:
   https://mixedinkey.com/harmonic-mixing-guide/
2. Mixed In Key, *The Camelot Wheel* (interactive): spellings, the basic
   and advanced moves (Energy Boost, Jaws Mix, Pay Attention, Power Block),
   the caution that a key change alone does not raise energy. Opened:
   https://mixedinkey.com/camelot-wheel/
3. Mixed In Key, *Energy Boost DJ Mixing Tutorial*: +2, one every 20 to 30
   minutes; -5 with rising Energy. Opened:
   https://mixedinkey.com/harmonic-mixing-guide/energy-boost-dj-mixing-tutorial/
4. Mixed In Key, *Jaws Mix* and *Pay Attention Mix*. Opened:
   https://mixedinkey.com/jaws-mix/ and https://mixedinkey.com/pay-attention-mix/
5. Mixed In Key, *Sorting your playlists by Energy Level* and *Power Block
   Mixing*: the 1 to 10 Energy Level, same-key blocks, 70/140 and 85/170
   bridges. Opened:
   https://mixedinkey.com/harmonic-mixing-guide/sorting-playlists-by-energy-level/
   and https://mixedinkey.com/harmonic-mixing-guide/power-block-dj-mixing-tutorial/
6. Mixed In Key (Yakov Vorobyev et al.), *Beyond Beatmatching* (online
   edition): ch. 2 (the Camelot wheel's invention by Mark Davis), ch. 8
   (consonant and clashing diagonals, +4, +7, +2, -2), ch. 10 (phrasing).
   Opened: https://mixedinkey.com/book/how-to-use-harmonic-mixing/ ,
   https://mixedinkey.com/book/use-advanced-harmonic-mixing-techniques/ ,
   https://mixedinkey.com/book/mix-a-flawless-dj-set/
7. Hendrik Schreiber (tagtraum industries), *Open Key Notation*
   (beaTunes). Opened: https://www.beatunes.com/en/open-key-notation.html

Technique and key-detection benchmarks:

8. Dan White, *Key Detection Software Comparison: 2015 Edition*, DJ
   TechTools, 16 November 2015. Opened:
   https://djtechtools.com/2015/11/16/key-detection-software-comparison-2015-edition/
9. Ean Golden and Jules Etro, *The Time Line of a Dance Floor*, DJ
   TechTools, 25 July 2010. Opened:
   https://djtechtools.com/2010/07/25/the-time-line-of-a-dance-floor/
10. P. Knees, Á. Faraldo, P. Herrera, R. Vogl, S. Böck, F. Hörschläger, M.
    Le Goff, *Two Data Sets for Tempo Estimation and Key Detection in
    Electronic Dance Music Annotated from User Corrections*, ISMIR 2015.
    Opened: https://mtg.upf.edu/system/files/publications/246_Paper.pdf
11. Á. Faraldo, *GiantSteps+ EDM Key Dataset* (Zenodo, 2017), and Faraldo,
    Jordà, Herrera, *A Multi-Profile Method for Key Estimation in EDM* (AES
    2017). Landing pages opened (https://zenodo.org/records/1095691 ,
    https://zenodo.org/records/3855499 ); full texts not opened.
12. H. Schreiber and M. Müller, *Musical Tempo and Key Estimation using
    Convolutional Neural Networks with Directional Filters*, SMC 2019.
    Opened: https://arxiv.org/abs/1903.10839

Not opened:

13. MIREX, *Audio Key Detection* evaluation (weights 1 exact, 0.5 fifth,
    0.3 relative, 0.2 parallel), as quoted in Knees et al. 2015; the wiki
    itself not opened.
14. Bill Brewster and Frank Broughton, *How to DJ Right* (Grove Press,
    2003); Stuart Soroka, *Harmonic Keys* (1986); Pioneer DJ's Master Tempo
    and fader ranges; zplane's élastique; Native Instruments' Traktor
    documentation; ITU-R BS.1770-4 (loudness; its K-weighting is pinned by
    this app's own Intensity tests).
15. Stefan Kostka and Dorothy Payne, *Tonal Harmony*; Edward Aldwell and
    Carl Schachter, *Harmony and Voice Leading* (the circle of fifths,
    scales, equal temperament).
