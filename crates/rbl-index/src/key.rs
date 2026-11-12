//! Ordering musical keys.
//!
//! Two orders, because rekordbox sorts the Key column by what it shows: the
//! classic names alphabetically (`A`, `Ab`, `Abm`, `Am`, `B`, …) and the
//! alphanumeric ones round the Camelot wheel (`1A`, `1B`, `2A`, …). The
//! general string fold drops `#` as punctuation, which put `F` and `F#` on
//! top of each other; keys are compared by their own rule instead.

use std::cmp::Ordering;

/// The wheel, `1A` to `12A` and `1B` to `12B`, as rekordbox spells the keys.
const MINOR: [&str; 12] = [
    "Abm", "Ebm", "Bbm", "Fm", "Cm", "Gm", "Dm", "Am", "Em", "Bm", "F#m", "Dbm",
];
const MAJOR: [&str; 12] = [
    "B", "F#", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E",
];

#[path = "key_spelling.rs"]
mod spelling;

/// Where a key sits on the Camelot wheel: `1A` is 0, `1B` is 1, `2A` is 2,
/// and so on to `12B` at 23. A name that is not a key — blank, or one the
/// wheel does not know — is `u32::MAX`, so it sorts after every key.
#[must_use]
pub fn camelot_rank(name: &str) -> u32 {
    spelling::rank(name).unwrap_or(u32::MAX)
}

/// Whether a stored key name is the key `wanted` names (wbx-57): the same
/// place on the Camelot wheel whatever either spelling, or, for a name off
/// the wheel, exactly the same text.
#[must_use]
pub fn same_key(stored: &str, wanted: &str) -> bool {
    match (spelling::rank(stored), spelling::rank(wanted)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => stored == wanted,
        _ => false,
    }
}

/// The Classic label shown by the interface; unknown values stay as stored.
fn displayed_name(name: &str) -> &str {
    let rank = camelot_rank(name);
    if rank >= 24 {
        return name;
    }
    let index = (rank / 2) as usize;
    if rank % 2 == 0 {
        MINOR[index]
    } else {
        MAJOR[index]
    }
}

/// Alphabetical, case-insensitive, and `#` counts: `F` before `F#` before
/// `F#m` before `Fm`. Blank last, as a missing value goes to the end.
#[must_use]
pub fn cmp_names(left: &str, right: &str) -> Ordering {
    let left = displayed_name(left);
    let right = displayed_name(right);
    match (left.is_empty(), right.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    let mut lhs = left.bytes().map(|byte| byte.to_ascii_lowercase());
    let mut rhs = right.bytes().map(|byte| byte.to_ascii_lowercase());
    loop {
        match (lhs.next(), rhs.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) if l == r => {}
            (Some(l), Some(r)) => return l.cmp(&r),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn file_spellings_follow_the_display_wheel() {
        for line in include_str!("../../../src/lib/keySpellings.fixture.tsv").lines() {
            let (stored, code) = line.split_once('\t').unwrap();
            assert_eq!(camelot_rank(stored), camelot_rank(code), "{stored}");
        }
        for unknown in [
            "Amazing",
            "Cminorish",
            "13m",
            "0d",
            "8AB",
            "Unknown",
            "H minor",
            "",
        ] {
            assert_eq!(camelot_rank(unknown), u32::MAX, "{unknown}");
        }
    }

    #[test]
    fn classic_order_follows_displayed_names_for_file_spellings() {
        assert_eq!(cmp_names("1d", "B"), Ordering::Greater); // displayed C, B
        assert_eq!(cmp_names("8A - Energy 6", "Am"), Ordering::Equal);
        assert_eq!(cmp_names("Amin", "C"), Ordering::Less);
    }

    #[test]
    fn the_wheel_goes_round_from_1a() {
        assert_eq!(camelot_rank("Abm"), 0);
        assert_eq!(camelot_rank("B"), 1);
        assert_eq!(camelot_rank("Ebm"), 2);
        assert_eq!(camelot_rank("E"), 23);
        assert_eq!(
            camelot_rank("G#m"),
            camelot_rank("Abm"),
            "an enharmonic spelling is the same key"
        );
        assert_eq!(
            camelot_rank("8A"),
            camelot_rank("Am"),
            "a code stored as a key"
        );
        assert_eq!(camelot_rank(""), u32::MAX);
        assert_eq!(camelot_rank("Unknown"), u32::MAX);
    }

    #[test]
    fn the_names_keep_their_sharps_apart() {
        let mut keys = vec!["Fm", "F#m", "F", "F#", "", "Ab", "A", "am"];
        keys.sort_by(|a, b| cmp_names(a, b));
        assert_eq!(keys, ["A", "Ab", "am", "F", "F#", "F#m", "Fm", ""]);
    }
}

/// Resulting key, using the spelling rules shared with the window.
#[must_use]
pub fn transpose(name: &str, semitones: i64) -> Option<String> {
    spelling::transpose(name, semitones)
}

#[cfg(test)]
#[path = "key_transpose_tests.rs"]
mod transpose_tests;
