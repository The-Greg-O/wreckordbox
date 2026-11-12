//! The closed Mutant edit vocabulary and its one derivation rule.
//! Cue positions are in the parent's timeline. Consumers scale them (and the
//! parent's grid) by `1 / tempo` only when assembling rendered analysis.
use crate::{protocol::ServiceError, tracks::TrackRecord};
use serde::{Deserialize, Serialize};

/// Stable refusals, also used as render-readiness reasons.
pub mod codes {
    pub const KEY_SHIFT_LIMIT: &str = "mutant_key_shift_limit";
    pub const TEMPO_LIMIT: &str = "mutant_tempo_limit";
    pub const STEMS_MISSING: &str = "mutant_stems_missing";
    pub const STEMS_STALE: &str = "mutant_stems_stale";
    pub const STEM_ALIGNMENT_UNKNOWN: &str = "stem_alignment_unknown";
    pub const INVALID_CUES: &str = "mutant_invalid_cues";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stem {
    Vocals,
    Melody,
    Bass,
    Drums,
}
impl Stem {
    pub const ALL: [Self; 4] = [Self::Vocals, Self::Melody, Self::Bass, Self::Drums];
    fn name(self) -> &'static str {
        match self {
            Self::Vocals => "vocals",
            Self::Melody => "melody",
            Self::Bass => "bass",
            Self::Drums => "drums",
        }
    }
}

/// A cue or loop, preserving rekordbox's raw cue kind and colours.
/// Kind 0 is Memory, 4 is Active Loop; 1..3, 5..17 are Hot Cues A..P.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub kind: u8,
    pub position_ms: u32,
    #[serde(default)]
    pub out_ms: u32,
    #[serde(default)]
    pub colour: u8,
    #[serde(default)]
    pub memory_colour: Option<u8>,
    #[serde(default)]
    pub comment: String,
}
impl Cue {
    fn label(&self) -> String {
        match self.kind {
            0 | 4 => format!("memory cue at {} ms", self.position_ms),
            k @ (1..=3 | 5..=17) => format!(
                "cue {}",
                char::from(b'A' + if k < 4 { k - 1 } else { k - 2 })
            ),
            k => format!("cue kind {k} at {} ms", self.position_ms),
        }
    }
}

/// Absent means inherit. All four Stems, zero shift and unit tempo normalize
/// to absence. `Some([])` cues explicitly removes every inherited cue.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stems: Option<Vec<Stem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_shift: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cues: Option<Vec<Cue>>,
}
impl Edits {
    pub fn validate(&mut self) -> Result<(), ServiceError> {
        if self.key_shift.is_some_and(|s| !(-6..=6).contains(&s)) {
            return Err(ServiceError::new(
                codes::KEY_SHIFT_LIMIT,
                "key shift must be within -6 to +6 semitones",
            ));
        }
        if self
            .tempo
            .is_some_and(|t| !t.is_finite() || !(0.84..=1.16).contains(&t))
        {
            return Err(ServiceError::new(
                codes::TEMPO_LIMIT,
                "tempo ratio must be within 0.84 to 1.16",
            ));
        }
        if self.key_shift == Some(0) {
            self.key_shift = None;
        }
        if self.tempo == Some(1.0) {
            self.tempo = None;
        }
        if let Some(kept) = &mut self.stems {
            kept.sort_unstable();
            if kept.is_empty() || kept.windows(2).any(|w| w[0] == w[1]) {
                return Err(ServiceError::new(
                    "bad_request",
                    "keep at least one Stem, with no duplicates",
                ));
            }
            if kept.len() == 4 {
                self.stems = None;
            }
        }
        Ok(())
    }
    /// Preserve legacy parent rows exactly, while validating every new or moved cue.
    pub fn validate_cues(
        &self,
        parent: Option<&TrackRecord>,
        inherited: &[Cue],
    ) -> Result<(), ServiceError> {
        let Some(cues) = &self.cues else {
            return Ok(());
        };
        let limit = parent
            .filter(|p| p.duration_seconds > 0)
            .map(|p| u64::from(p.duration_seconds) * 1000);
        let mut remaining = inherited.to_vec();
        for (index, cue) in cues.iter().enumerate() {
            let duplicate = !matches!(cue.kind, 0 | 4)
                && cues.iter().filter(|other| other.kind == cue.kind).count() > 1;
            // A real parent row may use an older/extended representation. Permit
            // each such row once, unchanged, including its original slot.
            if let Some(at) = remaining.iter().position(|old| old == cue) {
                remaining.remove(at);
                continue;
            }
            if cue.kind > 17
                || duplicate
                || (cue.out_ms != 0 && cue.out_ms <= cue.position_ms)
                || (cue.kind == 4 && cue.out_ms == 0)
                || cue
                    .memory_colour
                    .is_some_and(|c| c > 7 || !matches!(cue.kind, 0 | 4))
                || limit.is_some_and(|end| {
                    u64::from(cue.position_ms) > end || u64::from(cue.out_ms) > end
                })
            {
                return Err(ServiceError::new(codes::INVALID_CUES, format!("cue {} ({}): invalid kind, loop, colour, duplicate Hot Cue, or position beyond the parent's length", index + 1, cue.label())));
            }
        }
        Ok(())
    }
    /// Apply only supplied aspects; explicit clearing is a separate command.
    pub fn patch(&mut self, patch: Self) {
        if patch.stems.is_some() {
            self.stems = patch.stems;
        }
        if patch.key_shift.is_some() {
            self.key_shift = patch.key_shift;
        }
        if patch.tempo.is_some() {
            self.tempo = patch.tempo;
        }
        if patch.cues.is_some() {
            self.cues = patch.cues;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aspect {
    Stems,
    KeyShift,
    Tempo,
    Cues,
}
impl Edits {
    pub fn clear(&mut self, aspect: Aspect) {
        match aspect {
            Aspect::Stems => self.stems = None,
            Aspect::KeyShift => self.key_shift = None,
            Aspect::Tempo => self.tempo = None,
            Aspect::Cues => self.cues = None,
        }
    }
}

/// These are readiness checks, not a promise that decoding/encoding will succeed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renderability {
    pub renderable: bool,
    pub reason: Option<ServiceError>,
}
impl Renderability {
    pub(crate) fn ready() -> Self {
        Self {
            renderable: true,
            reason: None,
        }
    }
    pub(crate) fn blocked(error: ServiceError) -> Self {
        Self {
            renderable: false,
            reason: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Derived {
    pub description: String,
    /// Unknown/unparseable keys remain unknown after transposition.
    pub key: Option<String>,
    pub bpm: Option<f64>,
    pub duration_seconds: Option<f64>,
    /// Unscaled cues in parent time, inherited until explicitly replaced.
    pub cues: Vec<Cue>,
    /// Always the parent's grid; no Mutant can override it.
    pub grid_parent: String,
}

/// Shared by every caller: no I/O, no cached parent metadata.
pub fn derive(
    edits: &Edits,
    parent: Option<&TrackRecord>,
    parent_cues: &[Cue],
    parent_id: &str,
) -> Derived {
    let tempo = edits.tempo.unwrap_or(1.0);
    let mut differences = Vec::new();
    if let Some(kept) = &edits.stems {
        for stem in Stem::ALL {
            if !kept.contains(&stem) {
                differences.push(format!("no {}", stem.name()));
            }
        }
    }
    if let Some(shift) = edits.key_shift {
        differences.push(format!("{shift:+} st"));
    }
    if edits.tempo.is_some() {
        differences.push(format!("tempo {:+.2}%", (tempo - 1.0) * 100.0));
    }
    if let Some(cues) = &edits.cues {
        for cue in cues {
            if parent_cues.contains(cue) {
                continue;
            }
            let old = parent_cues.iter().find(|old| {
                old.kind == cue.kind
                    && (cue.kind != 0 && cue.kind != 4 || old.position_ms == cue.position_ms)
            });
            let change = match old {
                Some(old) if old.position_ms != cue.position_ms || old.out_ms != cue.out_ms => {
                    "moved"
                }
                Some(_) => "changed",
                None => "added",
            };
            differences.push(format!("{} {change}", cue.label()));
        }
        for cue in parent_cues {
            if !cues.iter().any(|new| {
                new.kind == cue.kind
                    && (cue.kind != 0 && cue.kind != 4 || new.position_ms == cue.position_ms)
            }) {
                differences.push(format!("{} removed", cue.label()));
            }
        }
    }
    Derived {
        description: if differences.is_empty() {
            "none".into()
        } else {
            differences.join(", ")
        },
        key: parent.and_then(|p| transpose(&p.key, edits.key_shift.unwrap_or(0))),
        bpm: parent.and_then(|p| p.bpm).map(|b| b * tempo),
        duration_seconds: parent
            .filter(|p| p.duration_seconds > 0)
            .map(|p| f64::from(p.duration_seconds) / tempo),
        cues: edits.cues.clone().unwrap_or_else(|| parent_cues.to_vec()),
        grid_parent: parent_id.into(),
    }
}

fn transpose(name: &str, shift: i64) -> Option<String> {
    rbl_index::key::transpose(name, shift)
}

/// Patch supplied aspects, preserving every other edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditMutant {
    pub id: Option<i64>,
    pub entry: Option<String>,
    pub edits: Edits,
    pub run: Option<i64>,
    pub source: Option<crate::facts::SourceArg>,
    pub description: Option<String>,
}
/// Return one aspect to its parent's value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearMutant {
    pub id: Option<i64>,
    pub entry: Option<String>,
    pub edit: Aspect,
    pub run: Option<i64>,
    pub source: Option<crate::facts::SourceArg>,
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;
    fn parent() -> TrackRecord {
        TrackRecord {
            id: "1".into(),
            title: "Parent".into(),
            artist: "DJ".into(),
            album: String::new(),
            genre: String::new(),
            label: String::new(),
            year: None,
            bpm: Some(120.0),
            key: "8A".into(),
            energy: Some(3),
            rating: 3,
            prep_colour: None,
            colour: None,
            colour_id: 0,
            comment: String::new(),
            duration_seconds: 120,
            date_added: None,
            play_count: 0,
            location: String::new(),
            file_name: String::new(),
            analysed: true,
            my_tags: Vec::new(),
            playlists: Vec::new(),
        }
    }
    fn cue(kind: u8, position_ms: u32) -> Cue {
        Cue {
            kind,
            position_ms,
            out_ms: 0,
            colour: 0,
            memory_colour: None,
            comment: String::new(),
        }
    }
    #[test]
    fn description_key_tempo_length_and_cues_share_one_derivation() {
        let cues = vec![cue(2, 1000), cue(0, 500)];
        let mut edited = cues.clone();
        edited[0].position_ms = 1500;
        let edits = Edits {
            stems: Some(vec![Stem::Melody, Stem::Bass, Stem::Drums]),
            key_shift: Some(1),
            tempo: Some(1.1),
            cues: Some(edited.clone()),
        };
        let derived = derive(&edits, Some(&parent()), &cues, "1");
        assert_eq!(
            derived.description,
            "no vocals, +1 st, tempo +10.00%, cue B moved"
        );
        assert_eq!(derived.key.as_deref(), Some("3A"));
        assert_eq!(derived.bpm, Some(132.0));
        assert!((derived.duration_seconds.unwrap() - 120.0 / 1.1).abs() < 1e-9);
        assert_eq!(derived.cues, edited); // Still parent time, never double-scaled.
        assert_eq!(derived.grid_parent, "1");
        let inherited = derive(&Edits::default(), Some(&parent()), &cues, "1");
        assert_eq!(inherited.description, "none");
        assert_eq!(inherited.key.as_deref(), Some("8A"));
        assert_eq!(inherited.duration_seconds, Some(120.0));
        assert_eq!(inherited.cues, cues);
        let removed = derive(
            &Edits {
                cues: Some(Vec::new()),
                ..Default::default()
            },
            Some(&parent()),
            &cues,
            "1",
        );
        assert_eq!(
            removed.description,
            "cue B removed, memory cue at 500 ms removed"
        );
        assert_eq!(removed.cues, []);
    }
    #[test]
    fn keys_wrap_preserve_mode_and_handle_unknown_values() {
        for (key, shift, wanted) in [
            ("Am", 1, "Bbm"),
            ("B♭", 1, "B"),
            ("F♯", -1, "F"),
            ("B", 1, "C"),
            ("Cm", -1, "Bm"),
            ("1A", -6, "7A"),
            ("8B", 6, "2B"),
            ("Db", 0, "Db"),
        ] {
            assert_eq!(transpose(key, shift).as_deref(), Some(wanted));
        }
        assert_eq!(transpose("", 1), None);
        assert_eq!(transpose("unknown", 2), None);
        assert_eq!(transpose("unknown", 0), None);
        for number in 1..=12 {
            for mode in ['A', 'B'] {
                for shift in -6..=6 {
                    let original = format!("{number}{mode}");
                    let shifted = transpose(&original, shift).unwrap();
                    assert_eq!(transpose(&shifted, -shift), Some(original));
                }
            }
        }
    }
    #[test]
    fn independent_clear_and_identity_values_inherit() {
        let mut edits = Edits {
            stems: Some(Stem::ALL.to_vec()),
            key_shift: Some(0),
            tempo: Some(1.0),
            cues: Some(vec![]),
        };
        edits.validate().unwrap();
        assert_eq!(
            edits,
            Edits {
                cues: Some(vec![]),
                ..Default::default()
            }
        );
        edits.clear(Aspect::Cues);
        assert_eq!(edits, Edits::default());
        let absent = derive(&edits, None, &[], "missing");
        assert_eq!(absent.key, None);
        assert_eq!(absent.bpm, None);
        assert_eq!(absent.duration_seconds, None);
    }
}
