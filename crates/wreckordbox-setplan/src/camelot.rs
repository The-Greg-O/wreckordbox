//! A key's place on the Camelot wheel, and how far apart two keys are.
use std::fmt;

/// `number` 1..=12 round the wheel; `major` is the B ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
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

    /// Moves between two keys: steps round the wheel (at most 6) plus one
    /// for changing ring. `8A`→`8A` 0, `8A`→`9A` or `8B` 1, `8A`→`9B` 2.
    #[must_use]
    pub fn moves(self, other: Self) -> u32 {
        let apart = u32::from(self.number.abs_diff(other.number));
        apart.min(12 - apart) + u32::from(self.major != other.major)
    }
}

impl fmt::Display for Camelot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.number, if self.major { 'B' } else { 'A' })
    }
}
