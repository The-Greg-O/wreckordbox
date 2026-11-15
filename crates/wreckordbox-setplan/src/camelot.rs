//! A key's place on the Camelot wheel, and how far apart two keys are.
use std::fmt;

/// `number` 1..=12 round the wheel; `major` is the B ring.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct Camelot {
    pub number: u8,
    pub major: bool,
}

impl Camelot {
    /// From the wheel rank `rbl_index::key::camelot_rank` gives: `1A` is 0,
    /// `1B` is 1, `2A` is 2, ... `12B` is 23. Anything else is no key.
    #[must_use]
    pub fn from_rank(rank: u32) -> Option<Self> {
        (rank < 24).then(|| Self {
            number: u8::try_from(rank / 2 + 1).unwrap_or(1),
            major: rank % 2 == 1,
        })
    }

    /// Reads `8A` / `8B` (Camelot), or a classic name (`Am`, `F#`, `Bbm`).
    /// The service reads the Library's keys with the index's own parser and
    /// [`Camelot::from_rank`]; this is for Recipes, tests and tools.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            let number: u8 = text[..digits].parse().ok()?;
            let major = match &text[digits..] {
                "A" | "a" => false,
                "B" | "b" => true,
                _ => return None,
            };
            return (1..=12).contains(&number).then_some(Self { number, major });
        }
        let mut chars = text.chars();
        let mut pitch: i32 = match chars.next()?.to_ascii_uppercase() {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return None,
        };
        let mut rest = chars.as_str();
        if let Some(stripped) = rest.strip_prefix('#') {
            pitch += 1;
            rest = stripped;
        } else if let Some(stripped) = rest.strip_prefix('b') {
            pitch -= 1;
            rest = stripped;
        }
        let minor = match rest {
            "" => false,
            "m" => true,
            _ => return None,
        };
        let pitch = usize::try_from(pitch.rem_euclid(12)).ok()?;
        let wheel: [u8; 12] = if minor {
            [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10]
        } else {
            [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1]
        };
        Some(Self {
            number: wheel[pitch],
            major: !minor,
        })
    }

    /// Moves between two keys, undirected: steps round the wheel (at most
    /// 6) plus one for changing ring. `8A`→`8A` 0, `8A`→`9A` or `8B` 1,
    /// `8A`→`9B` 2. The sequencer scores [`Camelot::step_to`] instead.
    #[must_use]
    pub fn moves(self, other: Self) -> u32 {
        let apart = u32::from(self.number.abs_diff(other.number));
        apart.min(12 - apart) + u32::from(self.major != other.major)
    }

    /// The directed move from this key to `other`, named and costed.
    #[must_use]
    pub fn step_to(self, other: Self) -> KeyStep {
        // Clockwise steps round the wheel, 0..12.
        let d = (i32::from(other.number) - i32::from(self.number)).rem_euclid(12);
        let (name, halves): (&'static str, u8) = match (self.major, other.major, d) {
            (a, b, 0) if a == b => ("same key", 0),
            (a, b, 1) if a == b => ("+1, a fifth up", 2),
            (a, b, 11) if a == b => ("-1, a fifth down", 2),
            (a, b, 2) if a == b => ("+2, energy boost", 4),
            (a, b, 10) if a == b => ("-2, wind-down", 4),
            (a, b, 3) if a == b => ("+3", 6),
            (a, b, 9) if a == b => ("-3", 6),
            (a, b, 4) if a == b => ("+4", 8),
            (a, b, 8) if a == b => ("-4", 8),
            (a, b, 7) if a == b => ("+7, a semitone up", 10),
            (a, b, 5) if a == b => ("-7, a semitone down", 10),
            (a, b, _) if a == b => ("tritone", 12),
            (false, true, 0) => ("relative major", 2),
            (true, false, 0) => ("relative minor", 2),
            (false, true, 11) | (true, false, 1) => ("consonant diagonal", 3),
            (false, true, 3) => ("parallel major", 4),
            (true, false, 9) => ("parallel minor", 4),
            (false, true, 1) | (true, false, 11) => ("clashing diagonal", 6),
            (false, true, 9) | (true, false, 3) => ("tritone ring flip", 12),
            // The rest of the ring changes: the shorter way round plus one
            // for the ring, as far as the worst.
            _ => {
                let apart = u8::try_from(d.min(12 - d)).unwrap_or(6);
                ("ring change", (2 * (apart + 1)).min(12))
            }
        };
        KeyStep {
            name,
            halves,
            from: self,
            to: other,
        }
    }

    /// The key a semitone shift of `semitones` makes: +1 semitone is +7 on
    /// the wheel, same ring.
    #[must_use]
    pub fn shifted(self, semitones: i32) -> Self {
        let number = (i32::from(self.number) - 1 + 7 * semitones).rem_euclid(12) + 1;
        Self {
            number: u8::try_from(number).unwrap_or(self.number),
            major: self.major,
        }
    }
}

/// A directed move between two keys: its name (Mixed In Key's where it has
/// one) and its cost, from 0 (same key) to 6 (a tritone; the tritone ring
/// flip). Costs: +1, -1 and the relative key 1; the consonant diagonal
/// 1.5; +2, -2 and the parallel key 2; the clashing diagonal and +3, -3 3;
/// +4, -4 4; the semitone (+7, -7) 5; the tritone and its ring flip 6.
/// `key.max_moves` admits a move whose cost is at most its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStep {
    name: &'static str,
    /// The cost in half moves, so it stays an integer.
    halves: u8,
    pub from: Camelot,
    pub to: Camelot,
}

impl KeyStep {
    #[must_use]
    pub fn name(self) -> &'static str {
        self.name
    }

    #[must_use]
    pub fn cost(self) -> f64 {
        f64::from(self.halves) / 2.0
    }

    /// +2 on the same ring: Mixed In Key's energy boost.
    #[must_use]
    pub fn is_boost(self) -> bool {
        self.halves == 4 && self.name.starts_with('+')
    }

    /// -2 on the same ring: a whole tone down.
    #[must_use]
    pub fn is_wind_down(self) -> bool {
        self.halves == 4 && self.name.starts_with('-')
    }
}

impl fmt::Display for Camelot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.number, if self.major { 'B' } else { 'A' })
    }
}
