# Set planning: a guide for agents

Read this before you write a Recipe or build a Set plan. It gives the
musical and arithmetic basis of harmonic mixing, tempo and energy flow, and
how each maps onto the Recipe that `wreckordbox set-plan build` reads. Terms
are as in the project's glossary: Recipe, Set plan, Situation, Intensity,
Energy. Print this guide with `wreckordbox set-plan guide` (`--json` gives
it in sections); it needs no service and reads nothing of the Library.

Every number below is derived in the text and recomputed by a test
(`crates/wreckordbox-setplan/tests/guide.rs`), so the document cannot drift
from the arithmetic. What is acoustics or music theory is derived; what is
convention is attributed by name; what could not be checked is marked
**unverified**. Everything here is a default the DJ overrides.

## 1. Keys

### Pitch classes, equal temperament, the circle of fifths

Number the twelve pitch classes C=0, C#/Db=1, D=2, Eb=3, E=4, F=5,
F#/Gb=6, G=7, Ab=8, A=9, Bb=10, B=11, arithmetic mod 12. In twelve-tone
equal temperament each semitone is a frequency ratio of 2^(1/12) (about
1.059463), so s semitones are 2^(s/12) and one semitone is 100 cents.

A major scale on tonic t is {t, t+2, t+4, t+5, t+7, t+9, t+11}. A natural
minor scale on tonic t uses the same seven pitch classes as the major scale
on t+3 (its relative major): A minor and C major are both the white keys.

Stepping the tonic up a perfect fifth (+7 semitones) changes exactly one
scale tone: the fourth degree rises a semitone (C major to G major: F
becomes F#). So keys a fifth apart share six of their seven tones. Twelve
steps of +7 visit all twelve pitch classes before returning (7 and 12 are
coprime), which closes the circle of fifths. In general, two major (or two
minor) keys d steps apart on the circle share this many scale tones:

<!-- table: shared-by-distance -->
| Steps apart on the circle (d) | Shared scale tones |
|---|---|
| 0 | 7 |
| 1 | 6 |
| 2 | 5 |
| 3 | 4 |
| 4 | 3 |
| 5 | 2 |
| 6 | 2 |

Sources: any standard harmony text covers the circle of fifths and equal
temperament (for example Kostka and Payne, *Tonal Harmony*; Aldwell and
Schachter, *Harmony and Voice Leading*). The table is computed from the
scale sets above.

### The Camelot wheel

The Camelot wheel (Mixed In Key's numbering) is the circle of fifths
relabelled: the numbers 1 to 12 go round the circle, one fifth up per step,
`A` is the minor ring and `B` the major ring, and `nA` is the relative
minor of `nB`. 8B is C major. So:

- major tonic of `nB` = 7 x (n - 8) mod 12
- minor tonic of `nA` = major tonic of `nB` - 3 mod 12 (8A is A minor)
- a key transposed up s semitones moves +7s mod 12 on the wheel, same ring;
  a wheel step of +k transposes by 7k mod 12 semitones (7 x 7 = 49 = 1 mod
  12, so 7 is its own inverse).

<!-- table: camelot -->
| Camelot | Minor key | Tonic | rekordbox | Camelot | Major key | Tonic | rekordbox |
|---|---|---|---|---|---|---|---|
| 1A | G#m / Abm | 8 | Abm | 1B | B | 11 | B |
| 2A | D#m / Ebm | 3 | Ebm | 2B | F# / Gb | 6 | F# |
| 3A | A#m / Bbm | 10 | Bbm | 3B | Db / C# | 1 | Db |
| 4A | Fm | 5 | Fm | 4B | Ab / G# | 8 | Ab |
| 5A | Cm | 0 | Cm | 5B | Eb / D# | 3 | Eb |
| 6A | Gm | 7 | Gm | 6B | Bb / A# | 10 | Bb |
| 7A | Dm | 2 | Dm | 7B | F | 5 | F |
| 8A | Am | 9 | Am | 8B | C | 0 | C |
| 9A | Em | 4 | Em | 9B | G | 7 | G |
| 10A | Bm | 11 | Bm | 10B | D | 2 | D |
| 11A | F#m / Gbm | 6 | F#m | 11B | A | 9 | A |
| 12A | C#m / Dbm | 1 | Dbm | 12B | E | 4 | E |

The rekordbox column is the Classic spelling this app shows and stores
(`rbl-index` `key.rs`); it was not checked against rekordbox itself here.
Mixed In Key's wheel and the 1A to 12B numbering are Mixed In Key's (*The
Camelot Wheel* and its harmonic mixing guide, mixedinkey.com). The app
also reads Open Key (`1m` = 8A, `1d` = 8B).

### Moves, from 8A (A minor)

Let a key be (n, ring). With `d = min(|n1 - n2|, 12 - |n1 - n2|)`, the
scale tones two keys share depend on d only: a relative pair has the same
seven tones, so changing ring changes the tonal centre, not the tone set.
"App moves" is what this app's sequencer counts (`Camelot::moves`: d plus
one for a change of ring) and "Key score" its contribution at weight 1,
`max(0, 1 - moves / 4)`.

<!-- table: moves -->
| Move | To | Arithmetic | Shared tones | App moves | Key score | Use |
|---|---|---|---|---|---|---|
| Same key | 8A | n | 7 | 0 | 1 | long blends |
| Fifth up (+1) | 9A | n + 1 | 6 | 1 | 0.75 | long blends |
| Fifth down (-1) | 7A | n - 1 | 6 | 1 | 0.75 | long blends |
| Relative major | 8B | n, A to B | 7 | 1 | 0.75 | long blends; mood brightens |
| Diagonal up | 9B | n + 1, A to B | 6 | 2 | 0.5 | blends |
| Diagonal down | 7B | n - 1, A to B | 6 | 2 | 0.5 | blends |
| Energy boost (+2) | 10A | n + 2 | 5 | 2 | 0.5 | short blends, cuts |
| Down two (-2) | 6A | n - 2 | 5 | 2 | 0.5 | short blends, cuts |
| Parallel major | 11B | n + 3, A to B | 4 | 4 | 0 | cuts |
| Semitone up (+7) | 3A | n + 7 | 2 | 5 | 0 | cuts only |
| Semitone down (-7, = +5) | 1A | n + 5 | 2 | 5 | 0 | cuts only |

Notes on the moves:

- **Same, +1/-1, A/B same number** are Mixed In Key's basic compatible
  moves (*harmonic mixing guide*). The shared-tone counts are derived.
- **+2, "energy boost"**: Mixed In Key's name. +2 on the wheel is two
  fifths up, which is a whole tone (2 semitones) up: A minor to B minor.
- **+7, one semitone up**: also described by Mixed In Key as an energy
  boost (**unverified** wording). It is +7 x 7 = 49 = 1 semitone; it shares
  only two tones, so it works as a cut or a drop into the new key, not as a
  blend. Pop calls the same device a key change up a semitone.
- **Diagonal**: going one number up or down while changing ring. Mixed In
  Key describes a diagonal move; which direction it recommends from each
  ring is **unverified**. Both directions share six tones (derived).
- **Parallel major/minor** (A minor to A major, 8A to 11B; 8B to 5A): same
  tonic, four shared tones. Strong colour change; a cut or a breakdown
  transition, not a long blend.
- Shared tones are a proxy. Two tracks in one key can still clash (bass
  lines, melodies off the scale), and tracks that sit on one chord blend
  more freely than the count says. Long blends of melodic parts want 7 or
  6 shared tones; percussive or vocal-only overlaps tolerate more.

## 2. Tempo

### Percent, BPM, and pitch

A tempo change is relative: from x to y BPM is (y - x) / x x 100 percent.
One BPM is 100 / x percent: 0.83% at 120, 0.78% at 128. Players' tempo
faders work in percent (Pioneer players offer ranges of 6, 10 and 16
percent and a wide range; exact ranges per model **unverified**).

Without key lock, a player changes tempo by playing faster, so every
frequency scales by the same factor and pitch moves by

    semitones = 12 x log2(1 + p / 100)

<!-- table: tempo-pitch -->
| Tempo change (%) | Pitch change (semitones) | Cents |
|---|---|---|
| 1 | 0.17 | 17 |
| 2 | 0.34 | 34 |
| 3 | 0.51 | 51 |
| 4 | 0.68 | 68 |
| 6 | 1.01 | 101 |
| 8 | 1.33 | 133 |
| 10 | 1.65 | 165 |
| 16 | 2.57 | 257 |
| -4 | -0.71 | -71 |
| -6 | -1.07 | -107 |

One semitone up is +5.95% (2^(1/12) - 1); one semitone down is -5.61%
(2^(-1/12) - 1). What this means for keys:

- A change of about +5.95% moves the track's key +7 on the wheel, about
  -5.61% moves it -7 (= +5). At 12% (2^(2/12) - 1 = 12.25%) it is +2.
- In between, the track is detuned between keys. Around 3% (half a
  semitone, about 50 cents) it is as far from its labelled key as it can
  be, and its labelled key no longer predicts what it clashes with. As a
  working rule, a tempo change of up to about 1.5% (26 cents) leaves the
  labelled key usable; the threshold is a judgement, not a measured value
  (**unverified**).
- Key lock (Pioneer's "Master Tempo") keeps pitch while changing tempo by
  time-stretching. Its artefacts (smeared transients, phasiness, a metallic
  sound on sustained tones) grow with the stretch; how much is acceptable
  depends on the material and the algorithm (**unverified** for any
  particular player).

### Which track moves, and half or double time

The DJ usually brings the incoming track to the tempo of the one playing.
The fader setting the incoming track needs is x / y - 1, not (y - x) / x:
from 120 to 126 BPM the change is +5.00% of the outgoing tempo, but the
incoming track is played at -4.76%. This app's sequencer measures the
change as a percent of the outgoing tempo.

A track at 70 BPM sits on a 140 BPM grid at double time (ratio 2), and 87
BPM against 174 likewise. Half or double time is a ratio of 2; this app's
sequencer does not treat it as a match (70 to 140 is a change of 100%).

How much change per transition and over a set is typical is convention,
not theory: small steps of a few percent per transition and a drift of
some 10 to 20 BPM over a long set are commonly described (**unverified**).

## 3. Energy and Intensity

- **Energy** is the DJ's own 1 to 5 judgement, stored as the track's star
  rating. It is not measured and the Recipe has no field for it; use it to
  choose the pool (`set-plan build ... --energy 3..5`).
- **Intensity** is a measurement (0 to 1) of how hard a track hits and how
  that changes over its length (`wreckordbox intensity measure`). The
  Recipe's `[intensity]` section works on the track's summary Intensity
  (the mean of its curve). It has not yet been calibrated against Energy;
  treat its scale as relative within one Library.

### Set shapes as Intensity curves

The shape is a list of points `{ at, intensity }`, `at` from 0 (first
track) to 1 (last), joined by straight lines and flat outside the first
and last points. Position i (from 0) of a set of n tracks is at i / (n - 1).

| Shape | Points (at, intensity) |
|---|---|
| Warm-up | (0, 0.25), (0.6, 0.45), (1, 0.6) |
| Build to peak | (0, 0.4), (0.8, 0.9), (1, 0.85) |
| Peak with waves | (0, 0.7), (0.3, 0.85), (0.45, 0.75), (0.75, 0.95), (0.85, 0.8), (1, 0.9) |
| Release / closing | (0, 0.85), (0.3, 0.9), (1, 0.35) |

These shapes are conventions of club DJing (a warm-up stays below the
headliner, a peak breathes in waves, a closing set comes down), not theory.

### Step size between neighbours

Between neighbours the shape asks for a step of slope x 1 / (n - 1),
where slope is the change in Intensity per unit of `at`. If
`intensity.max_step` is below the steepest segment's step, the shape
cannot be followed exactly and the plan will lag behind it. So choose

    max_step >= max over segments of |intensity2 - intensity1| / (at2 - at1) / (n - 1)

plus room for the pool not having the exact values. Example: a warm-up of
12 tracks with the shape above has steepest slope 0.15 / 0.4 = 0.375 (its
last segment), so a step of 0.0341 per track; `max_step = 0.15` leaves
plenty of room. Section 4 checks each example Recipe this way.

### Key moves and perceived energy

Conventional, not derived (Mixed In Key's guide among others): moving up
the wheel (+1, +2, +7) is heard as a lift, down as a relaxation; minor to
relative major as brighter, major to minor as darker. This app scores key
moves by distance only, without direction.

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
| `intensity.shape` | the curve above | |
| `intensity.max_step` | most Intensity change between neighbours, above 0, at most 1 | yes |
| `intensity.missing` | `"leave_out"` (default) or `"allow"` a track with no Intensity; `"allow"` cannot go with `max_step` | |
| `tempo.max_change` | most tempo change in percent of the outgoing tempo, above 0 | yes |
| `key.max_moves` | most app moves on the wheel, 0 to 7 | yes |
| `situations.place` | `{ situation, from, to, require }`: where a Situation belongs, as fractions; `require = true` makes every track there carry it | `require` only |
| `recency.half_life_days`, `recency.as_of` | favour recent additions; `as_of` YYYY-MM-DD, default the newest addition in the pool | |
| `genre.mix` | `"vary"` (neighbours of different genres) or `"stay"` | |
| `<section>.weight` | how much the criterion counts, 0 or more, default 1 | |

Situation names must be the Tag schema's `situation` terms when the
Library has a schema; `set-plan check` lists them when one is wrong.

What `key.max_moves` admits, by the moves table: 0 = same key only; 1 = the
basic moves (+1, -1, relative); 2 adds the diagonal and +2/-2; the
semitone moves (+7) need 5, which also admits nearly everything else. A
Recipe cannot ask for "basic moves or an energy boost only".

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

Hard limits are never broken in a returned plan; weights only rank the
plans that keep them. The search is a beam of 48 partial orders, so under
very tight limits it can report a Recipe unsatisfiable when a plan exists;
loosen the limit it names. Missing data is named: a track without
Intensity (unless `missing = "allow"`), without tempo under a tempo limit
or without key under a key limit is listed in `left_out` with what it
lacks; a criterion scored without data gives 0 and says why.

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

The reasons read "Intensity 0.62 where the shape wants 0.60, ...", "8A to
9A, 1 move" and "124.00 to 126.00 BPM, +1.6%". A low criterion in a good
plan is the cost of keeping another; an unsatisfiable Recipe names the
criterion in the way, the limits that alone block it, and the position it
stopped at.

### Example Recipes

Save one to a file, check it, then build from a pool:

    wreckordbox set-plan check warm-up.toml
    wreckordbox set-plan build warm-up.toml --playlist "Warm-up pool"
    wreckordbox set-plan show ID --json

The Situation names are examples: use your Tag schema's terms, or leave the
section out.

A warm-up set: low and rising, small tempo steps, strict keys, one genre.

```toml
version = 1
name = "Warm-up"
tracks = 12

[intensity]
shape = [{ at = 0.0, intensity = 0.25 }, { at = 0.6, intensity = 0.45 }, { at = 1.0, intensity = 0.6 }]
max_step = 0.15

[tempo]
max_change = 3.0

[key]
max_moves = 1
weight = 1.5

[situations]
place = [{ situation = "opener", from = 0.0, to = 0.0 }]

[genre]
mix = "stay"
weight = 0.5
```

A peak-time hour: 16 tracks (an hour at about 3.75 minutes a track), high
with waves, energy boosts allowed, varied genres, recent tracks favoured.

```toml
version = 1
name = "Peak-time hour"
tracks = 16

[intensity]
shape = [
  { at = 0.0, intensity = 0.7 },
  { at = 0.3, intensity = 0.85 },
  { at = 0.45, intensity = 0.75 },
  { at = 0.75, intensity = 0.95 },
  { at = 0.85, intensity = 0.8 },
  { at = 1.0, intensity = 0.9 },
]
max_step = 0.2

[tempo]
max_change = 4.0

[key]
max_moves = 2

[recency]
half_life_days = 60
weight = 0.5

[genre]
mix = "vary"
weight = 0.3
```

A closing set: holds the peak briefly, then comes down; a closer at the end.

```toml
version = 1
name = "Closing"
tracks = 10

[intensity]
shape = [{ at = 0.0, intensity = 0.85 }, { at = 0.3, intensity = 0.9 }, { at = 1.0, intensity = 0.35 }]
max_step = 0.2

[tempo]
max_change = 3.0

[key]
max_moves = 1
weight = 1.5

[situations]
place = [{ situation = "closer", from = 0.9, to = 1.0 }]
```

Each example's steepest shape segment against its `max_step` (section 3's
rule; slope in Intensity per unit of `at`, step = slope / (tracks - 1)):

<!-- table: steepest-step -->
| Recipe | Tracks | Steepest slope | Step per track | max_step |
|---|---|---|---|---|
| Warm-up | 12 | 0.375 | 0.0341 | 0.15 |
| Peak-time hour | 16 | 1.5 | 0.1 | 0.2 |
| Closing | 10 | 0.7857 | 0.0873 | 0.2 |

The warm-up's steepest segment is the last (0.45 to 0.6 over 0.4), the
peak hour's the drop from 0.95 to 0.8 over 0.1, the closing set's the
descent from 0.9 to 0.35 over 0.7.

## 5. Limits and honesty

- **Derived** (acoustics, equal temperament, set arithmetic): the
  semitone ratio, shared scale tones, the Camelot arithmetic, the
  tempo-to-pitch formula, every score in section 4.
- **Convention**: which moves are "compatible" and what they feel like
  (Mixed In Key's harmonic mixing guide and Camelot wheel), set shapes,
  typical tempo changes. Treat them as defaults.
- **Key detection is unreliable** in places: relative major/minor
  confusions (same tones, so detectors often swap 8A and 8B), fifth errors,
  tracks that modulate or are atonal (drums, spoken word), and detuned
  recordings that sit between keys. A wrong label makes a "compatible"
  move clash; the DJ's ear and the DJ's corrections win.
- **The app's key arithmetic** counts a ring change as a move, so the
  diagonal (six shared tones) costs the same as +2 (five), and the relative
  move (seven) the same as +1 (six). It ignores direction, so +2 and -2
  score the same. It scores the stored key and does not shift it for the
  tempo change.
- **Intensity** is not yet calibrated against Energy.
- These are defaults. The DJ overrides any of them.
