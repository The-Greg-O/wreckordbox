#![allow(clippy::unwrap_used)]
use super::{camelot_rank, transpose};

#[test]
fn shared_spelling_fixture_survives_transposition() {
    for line in include_str!("../../../src/lib/keySpellings.fixture.tsv").lines() {
        let (raw, camelot) = line.split_once('\t').unwrap();
        for shift in -6..=6 {
            assert_eq!(
                transpose(raw, shift).map(|s| camelot_rank(&s)),
                transpose(camelot, shift).map(|s| camelot_rank(&s)),
                "{raw}, {shift}"
            );
        }
    }
    for unknown in ["", "unknown", "♭", "♯", "8Ａ", "13A"] {
        for shift in -6..=6 {
            assert_eq!(transpose(unknown, shift), None);
        }
    }
}

#[test]
fn every_transposition_matches_literal_chromatic_tables() {
    // Independent tables indexed by pitch C through B, not wheel adjacency.
    let minor = [
        "5A", "12A", "7A", "2A", "9A", "4A", "11A", "6A", "1A", "8A", "3A", "10A",
    ];
    let major = [
        "8B", "3B", "10B", "5B", "12B", "7B", "2B", "9B", "4B", "11B", "6B", "1B",
    ];
    let classic_minor = [
        "Cm", "Dbm", "Dm", "Ebm", "Em", "Fm", "F#m", "Gm", "Abm", "Am", "Bbm", "Bm",
    ];
    let classic_major = [
        "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ];
    for table in [minor, major, classic_minor, classic_major] {
        for (pitch, key) in table.iter().enumerate() {
            for shift in -6..=6 {
                let target =
                    usize::try_from((i64::try_from(pitch).unwrap() + shift).rem_euclid(12))
                        .unwrap();
                assert_eq!(
                    transpose(key, shift).as_deref(),
                    Some(table[target]),
                    "{key}, {shift}"
                );
            }
        }
    }
}
