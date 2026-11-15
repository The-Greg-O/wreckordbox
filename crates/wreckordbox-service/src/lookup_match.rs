//! Whether a lookup candidate is the track: the rules `lookup.check`
//! answers by (wbx-37). Pure: two descriptions in, a verdict and its
//! findings out. Nothing here reads, writes or fetches anything.
//!
//! The comparison, field by field, after [`normalise`] (titles and mix
//! names after [`title_key`], which also drops apostrophes):
//!
//! | Field | Same | Held for review | Rejected |
//! |---|---|---|---|
//! | title | the same | | different |
//! | artists | the same names, in any order | some names shared, not all | no name shared |
//! | mix name | the same; none and "Original Mix" are the same | different, or one side has none and the other is not the original | |
//! | remaster | neither is one, or both are (of the same year when both say) | only one is, or of another year | |
//! | duration | within the tolerance | outside it, or either side does not say | |
//! | release year | the same, or either side does not say | different | |
//! | label | the same, or either side does not say | different | |
//!
//! The verdict is the worst finding: one rejection rejects, otherwise one
//! hold holds, otherwise the candidate is accepted. A mix name written at
//! the end of the title, in brackets or after a spaced dash ("Night Signal
//! (Extended Mix)", "Night Signal - Extended Mix"), is read as the mix
//! name when that side gives none of its own; a remaster ending ("- 2019
//! Remaster", "(Remastered)") is read as the remaster, not the mix.

use serde::{Deserialize, Serialize};

/// The default tolerance between the track's duration and the
/// candidate's, in seconds. The Library keeps whole seconds and shops show
/// whole seconds, each rounded its own way, and a shop's preview of the
/// same master can differ by a second or two at its ends.
pub const DEFAULT_TOLERANCE_SECONDS: f64 = 3.0;

/// What the check gives its choice as a probability: fixed, not measured,
/// as the rule-based Decider's normalised match is. The rules here are
/// deterministic and will look over-sure wherever they are wrong.
pub const CHECK_PROBABILITY: f64 = 0.9;

/// What a check concludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The same recording: its fields may become Facts.
    Accepted,
    /// Perhaps the same recording, perhaps another mix, edit or master:
    /// for the DJ to look at. Nothing becomes a Fact.
    Held,
    /// Another recording.
    Rejected,
}

impl Verdict {
    /// The name of the verdict, as a Decision's candidate.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Held => "held",
            Self::Rejected => "rejected",
        }
    }
}

/// One side of the comparison: the track as the Library has it, or the
/// candidate as the agent found it. Empty text and `None` mean "does not
/// say".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Side {
    pub artist: String,
    pub title: String,
    pub mix_name: String,
    pub duration_seconds: Option<f64>,
    pub year: Option<u16>,
    pub label: String,
}

/// What one field's comparison found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// `title`, `artist`, `mix_name`, `duration`, `release` or `label`.
    pub field: String,
    /// The verdict this field alone would give.
    pub verdict: Verdict,
    /// What was compared and what was found, in words.
    pub message: String,
}

/// What [`check`] answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub verdict: Verdict,
    /// Every field compared, in the order of the table above.
    pub findings: Vec<Finding>,
}

impl Checked {
    /// Why, in a sentence or two: the findings that decided the verdict;
    /// for an accepted candidate, what matched.
    #[must_use]
    pub fn reason(&self) -> String {
        let deciding: Vec<&str> = self
            .findings
            .iter()
            .filter(|finding| finding.verdict == self.verdict)
            .map(|finding| finding.message.as_str())
            .collect();
        deciding.join("; ")
    }
}

/// Lower-case, every run of characters that are not letters or digits made
/// one space, trimmed: as the rule-based Decider normalises
/// ([`crate::decider::normalise`]).
#[must_use]
pub fn normalise(text: &str) -> String {
    crate::decider::normalise(text)
}

/// The artist names a credit names: split at commas, ampersands,
/// semicolons, slashes, "feat.", "ft.", "featuring", "vs." and " x ".
/// Trimmed, empty pieces left out, in the credit's order.
#[must_use]
pub fn artists(credit: &str) -> Vec<String> {
    let mut lowered = format!(" {} ", credit.to_lowercase());
    for joiner in [
        " featuring ",
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " vs. ",
        " vs ",
        " x ",
    ] {
        lowered = lowered.replace(joiner, ",");
    }
    // Byte offsets of the lowered text are not the credit's in general, so
    // the names come from the lowered text; only their case is lost, and
    // the description gives them back in the credit's own spelling when it
    // can.
    let names: Vec<String> = lowered
        .split([',', '&', ';', '/'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    names
        .into_iter()
        .map(|name| original_spelling(credit, &name).unwrap_or(name))
        .collect()
}

/// `name` (lower-case) as `credit` spells it, when it appears there.
fn original_spelling(credit: &str, name: &str) -> Option<String> {
    let lowered = credit.to_lowercase();
    if lowered.len() != credit.len() {
        return None;
    }
    let at = lowered.find(name)?;
    credit.get(at..at + name.len()).map(str::to_owned)
}

/// Words that mark a bracketed end of a title, or an end after a spaced
/// dash, as its mix name.
const MIX_WORDS: &[&str] = &[
    "mix",
    "remix",
    "edit",
    "dub",
    "version",
    "rework",
    "vip",
    "bootleg",
    "instrumental",
    "remaster",
    "remastered",
];

/// Words that mark such an end as a remaster: the same mix, another master.
const REMASTER_WORDS: &[&str] = &["remaster", "remastered"];

/// The dashes a title's suffix may follow, with a space each side.
const DASHES: &[&str] = &[" - ", " \u{2013} ", " \u{2014} "];

fn has_word(text: &str, words: &[&str]) -> bool {
    normalise(text).split(' ').any(|word| words.contains(&word))
}

/// Whether a mix name or title suffix names a remaster.
fn is_remaster(text: &str) -> bool {
    has_word(text, REMASTER_WORDS)
}

/// A title as it is compared: apostrophes dropped (so "Don't" and "Dont"
/// are one word), then [`normalise`]d.
#[must_use]
pub fn title_key(text: &str) -> String {
    let joined: String = text
        .chars()
        .filter(|c| !matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '`' | '\u{02bc}'))
        .collect();
    normalise(&joined)
}

/// A mix-name ending of a title: "(Extended Mix)", "[VIP Remix]",
/// " - 2019 Remaster". The rest of the title and the ending.
fn suffix(title: &str) -> Option<(String, String)> {
    let trimmed = title.trim();
    for (open, close) in [('(', ')'), ('[', ']')] {
        if !trimmed.ends_with(close) {
            continue;
        }
        let Some(at) = trimmed.rfind(open) else {
            continue;
        };
        let inside = &trimmed[at + open.len_utf8()..trimmed.len() - close.len_utf8()];
        if has_word(inside, MIX_WORDS) {
            return Some((trimmed[..at].trim().to_owned(), inside.trim().to_owned()));
        }
    }
    let (at, dash) = DASHES
        .iter()
        .filter_map(|dash| trimmed.rfind(dash).map(|at| (at, *dash)))
        .max_by_key(|(at, _)| *at)?;
    let after = &trimmed[at + dash.len()..];
    (has_word(after, MIX_WORDS) && !trimmed[..at].trim().is_empty())
        .then(|| (trimmed[..at].trim().to_owned(), after.trim().to_owned()))
}

/// The title without a mix name at its end, and that mix name:
/// "Night Signal (Extended Mix)" and "Night Signal - Extended Mix" are
/// both ("Night Signal", "Extended Mix").
#[must_use]
pub fn split_title(title: &str) -> (String, Option<String>) {
    suffix(title).map_or_else(
        || (title.trim().to_owned(), None),
        |(rest, ending)| (rest, Some(ending)),
    )
}

/// The title and mix name of a side, with a mix name in the title moved
/// out when the side gives none of its own.
fn title_and_mix(title: &str, mix_name: &str) -> (String, String) {
    let (bare, in_title) = split_title(title);
    if mix_name.trim().is_empty() {
        if let Some(mix) = in_title {
            return (bare, mix);
        }
        return (title.trim().to_owned(), String::new());
    }
    // Both given: compare the title without its ending only when the
    // ending says the mix name again.
    match in_title {
        Some(mix) if title_key(&mix) == title_key(mix_name) => (bare, mix_name.trim().to_owned()),
        _ => (title.trim().to_owned(), mix_name.trim().to_owned()),
    }
}

/// A side's title, mix name and remaster, each taken out of the others:
/// a remaster ending ("- 2019 Remaster", "(Remastered)") or a mix name
/// that only says remaster is the remaster, not the mix.
struct Parsed {
    title: String,
    mix: String,
    remaster: String,
}

fn parse(side: &Side) -> Parsed {
    let mut title = side.title.trim().to_owned();
    let mut remaster = String::new();
    if let Some((rest, ending)) = suffix(&title) {
        if is_remaster(&ending) {
            title = rest;
            remaster = ending;
        }
    }
    let mut mix_name = side.mix_name.trim().to_owned();
    if remaster.is_empty() && is_remaster(&mix_name) {
        remaster = std::mem::take(&mut mix_name);
    }
    let (title, mut mix) = title_and_mix(&title, &mix_name);
    if remaster.is_empty() && is_remaster(&mix) {
        remaster = std::mem::take(&mut mix);
    }
    Parsed {
        title,
        mix,
        remaster,
    }
}

/// Whether two mix names are the same mix: equal once compared as titles
/// are, or both the original. For a caller with two names for one field.
#[must_use]
pub fn same_mix(one: &str, other: &str) -> bool {
    let (one, other) = (title_key(one), title_key(other));
    one == other || (original(&one) && original(&other))
}

/// Whether a normalised mix name is the original mix: none, "original" or
/// "original mix".
fn original(mix: &str) -> bool {
    matches!(mix, "" | "original" | "original mix")
}

/// Compares the candidate with the track. `tolerance_seconds` is how far
/// apart the durations may be.
#[must_use]
pub fn check(track: &Side, candidate: &Side, tolerance_seconds: f64) -> Checked {
    let ours = parse(track);
    let theirs = parse(candidate);
    let findings = vec![
        compare_title(&ours.title, &theirs.title),
        compare_artists(&track.artist, &candidate.artist),
        compare_mix(&ours.mix, &theirs.mix),
        compare_remaster(&ours.remaster, &theirs.remaster),
        compare_duration(
            track.duration_seconds,
            candidate.duration_seconds,
            tolerance_seconds,
        ),
        compare_year(track.year, candidate.year),
        compare_label(&track.label, &candidate.label),
    ];
    let verdict = findings
        .iter()
        .map(|finding| finding.verdict)
        .max()
        .unwrap_or(Verdict::Accepted);
    Checked { verdict, findings }
}

fn finding(field: &str, verdict: Verdict, message: String) -> Finding {
    Finding {
        field: field.to_owned(),
        verdict,
        message,
    }
}

fn compare_title(track: &str, candidate: &str) -> Finding {
    if title_key(track) == title_key(candidate) {
        finding(
            "title",
            Verdict::Accepted,
            format!("the title is the same (\"{candidate}\")"),
        )
    } else {
        finding(
            "title",
            Verdict::Rejected,
            format!("the title is \"{candidate}\", not \"{track}\""),
        )
    }
}

fn compare_artists(track: &str, candidate: &str) -> Finding {
    let ours: Vec<String> = artists(track).iter().map(|a| normalise(a)).collect();
    let theirs: Vec<String> = artists(candidate).iter().map(|a| normalise(a)).collect();
    let shared = theirs.iter().filter(|name| ours.contains(name)).count();
    let mut sorted_ours = ours.clone();
    sorted_ours.sort();
    sorted_ours.dedup();
    let mut sorted_theirs = theirs.clone();
    sorted_theirs.sort();
    sorted_theirs.dedup();
    if !sorted_ours.is_empty() && sorted_ours == sorted_theirs {
        finding(
            "artist",
            Verdict::Accepted,
            format!("the artists are the same (\"{candidate}\")"),
        )
    } else if shared > 0 {
        finding(
            "artist",
            Verdict::Held,
            format!("the artists are \"{candidate}\", not all the same as \"{track}\""),
        )
    } else {
        finding(
            "artist",
            Verdict::Rejected,
            format!("the artist is \"{candidate}\", not \"{track}\""),
        )
    }
}

fn compare_mix(track: &str, candidate: &str) -> Finding {
    let shown = |mix: &str| {
        if mix.is_empty() {
            "none".to_owned()
        } else {
            format!("\"{mix}\"")
        }
    };
    if same_mix(track, candidate) {
        finding(
            "mix_name",
            Verdict::Accepted,
            format!("the mix name is the same ({})", shown(candidate)),
        )
    } else {
        finding(
            "mix_name",
            Verdict::Held,
            format!(
                "the mix name is {}, not {}: perhaps another mix",
                shown(candidate),
                shown(track)
            ),
        )
    }
}

fn compare_remaster(track: &str, candidate: &str) -> Finding {
    let year = |text: &str| -> String { text.chars().filter(char::is_ascii_digit).collect() };
    match (track.is_empty(), candidate.is_empty()) {
        (true, true) => finding(
            "remaster",
            Verdict::Accepted,
            "neither is a remaster".to_owned(),
        ),
        (false, false) if year(track).is_empty() || year(candidate).is_empty() || year(track) == year(candidate) => finding(
            "remaster",
            Verdict::Accepted,
            format!("both are the remaster (\"{candidate}\")"),
        ),
        (false, false) => finding(
            "remaster",
            Verdict::Held,
            format!("the candidate is the remaster \"{candidate}\", the track \"{track}\": perhaps another master"),
        ),
        (true, false) => finding(
            "remaster",
            Verdict::Held,
            format!("the candidate is a remaster (\"{candidate}\") and the track is not: perhaps another master"),
        ),
        (false, true) => finding(
            "remaster",
            Verdict::Held,
            format!("the track is a remaster (\"{track}\") and the candidate is not: perhaps another master"),
        ),
    }
}

fn compare_duration(track: Option<f64>, candidate: Option<f64>, tolerance: f64) -> Finding {
    match (track.filter(|seconds| *seconds > 0.0), candidate) {
        (Some(ours), Some(theirs)) => {
            let apart = (ours - theirs).abs();
            if apart <= tolerance {
                finding(
                    "duration",
                    Verdict::Accepted,
                    format!(
                        "the duration is within {tolerance} s ({} against {})",
                        clock(theirs),
                        clock(ours)
                    ),
                )
            } else {
                finding(
                    "duration",
                    Verdict::Held,
                    format!(
                        "the duration is {}, not {}: {apart:.0} s apart, more than the {tolerance} s allowed; perhaps another edit or master",
                        clock(theirs),
                        clock(ours)
                    ),
                )
            }
        }
        (None, _) => finding(
            "duration",
            Verdict::Held,
            "the Library does not know the track's duration, so it cannot be compared".to_owned(),
        ),
        (_, None) => finding(
            "duration",
            Verdict::Held,
            "the candidate gives no duration, so it cannot be compared".to_owned(),
        ),
    }
}

fn compare_year(track: Option<u16>, candidate: Option<u16>) -> Finding {
    match (track, candidate) {
        (Some(ours), Some(theirs)) if ours != theirs => finding(
            "release",
            Verdict::Held,
            format!("released in {theirs}, not {ours}: perhaps a reissue or a remaster"),
        ),
        (Some(ours), Some(_)) => finding(
            "release",
            Verdict::Accepted,
            format!("released in the same year ({ours})"),
        ),
        _ => finding(
            "release",
            Verdict::Accepted,
            "the release year is not known on both sides; not compared".to_owned(),
        ),
    }
}

fn compare_label(track: &str, candidate: &str) -> Finding {
    let ours = normalise(track);
    let theirs = normalise(candidate);
    if ours.is_empty() || theirs.is_empty() {
        finding(
            "label",
            Verdict::Accepted,
            "the label is not known on both sides; not compared".to_owned(),
        )
    } else if ours == theirs {
        finding(
            "label",
            Verdict::Accepted,
            format!("the label is the same (\"{candidate}\")"),
        )
    } else {
        finding(
            "label",
            Verdict::Held,
            format!("the label is \"{candidate}\", not \"{track}\": perhaps another release"),
        )
    }
}

/// Seconds as `m:ss`.
fn clock(seconds: f64) -> String {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a duration is a few thousand seconds at most"
    )]
    let whole = seconds.round().max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// A duration as a candidate gives it: seconds as a number, or text in
/// `m:ss` or `h:mm:ss`, or seconds as text. `None` when it is none of them.
#[must_use]
pub fn duration_of(given: &serde_json::Value) -> Option<f64> {
    match given {
        serde_json::Value::Number(number) => number.as_f64().filter(|s| s.is_finite() && *s >= 0.0),
        serde_json::Value::String(text) => {
            let text = text.trim();
            if let Ok(seconds) = text.parse::<f64>() {
                return (seconds.is_finite() && seconds >= 0.0).then_some(seconds);
            }
            let parts: Vec<&str> = text.split(':').collect();
            if !(2..=3).contains(&parts.len()) {
                return None;
            }
            let mut total = 0.0;
            for (at, part) in parts.iter().enumerate() {
                let value: f64 = part.parse().ok()?;
                if value < 0.0 || (at > 0 && value >= 60.0) {
                    return None;
                }
                total = total * 60.0 + value;
            }
            Some(total)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Side {
        Side {
            artist: "Kessler & Vane".to_owned(),
            title: "Night Signal".to_owned(),
            mix_name: "Extended Mix".to_owned(),
            duration_seconds: Some(372.0),
            year: Some(2019),
            label: "Lowfield".to_owned(),
        }
    }

    fn verdict(candidate: &Side) -> Verdict {
        check(&track(), candidate, DEFAULT_TOLERANCE_SECONDS).verdict
    }

    #[test]
    fn the_same_recording_spelled_another_way_is_accepted() {
        let candidate = Side {
            artist: "Vane, Kessler".to_owned(),
            title: "Night Signal (Extended Mix)".to_owned(),
            mix_name: String::new(),
            duration_seconds: Some(374.0),
            year: None,
            label: "LOWFIELD".to_owned(),
        };
        assert_eq!(verdict(&candidate), Verdict::Accepted);
    }

    #[test]
    fn another_mix_or_a_duration_outside_the_tolerance_is_held() {
        let mut radio = track();
        radio.mix_name = "Radio Edit".to_owned();
        assert_eq!(verdict(&radio), Verdict::Held);

        let mut long = track();
        long.duration_seconds = Some(376.0);
        let checked = check(&track(), &long, DEFAULT_TOLERANCE_SECONDS);
        assert_eq!(checked.verdict, Verdict::Held);
        assert!(
            checked.reason().contains("6:16, not 6:12"),
            "{}",
            checked.reason()
        );

        let mut unknown = track();
        unknown.duration_seconds = None;
        assert_eq!(verdict(&unknown), Verdict::Held);

        let mut reissue = track();
        reissue.year = Some(2023);
        assert_eq!(verdict(&reissue), Verdict::Held);

        let mut one_of_two = track();
        one_of_two.artist = "Kessler".to_owned();
        assert_eq!(verdict(&one_of_two), Verdict::Held);
    }

    #[test]
    fn a_track_with_no_mix_name_matches_only_the_original() {
        let mut ours = track();
        ours.mix_name = String::new();
        let mut original = track();
        original.mix_name = "Original Mix".to_owned();
        assert_eq!(check(&ours, &original, 3.0).verdict, Verdict::Accepted);
        assert_eq!(check(&ours, &track(), 3.0).verdict, Verdict::Held);
    }

    #[test]
    fn a_different_title_or_artist_is_rejected_with_the_reason() {
        let mut other = track();
        other.title = "Day Signal".to_owned();
        let checked = check(&track(), &other, 3.0);
        assert_eq!(checked.verdict, Verdict::Rejected);
        assert_eq!(
            checked.reason(),
            "the title is \"Day Signal\", not \"Night Signal\""
        );

        let mut stranger = track();
        stranger.artist = "Someone Else".to_owned();
        assert_eq!(verdict(&stranger), Verdict::Rejected);
    }

    #[test]
    fn apostrophes_and_trivial_punctuation_do_not_make_another_title() {
        let mut ours = track();
        ours.title = "Don't Stop".to_owned();
        for theirs in [
            "Dont Stop",
            "Don\u{2019}t Stop",
            "Don't Stop!",
            "DON'T  STOP",
        ] {
            let mut candidate = ours.clone();
            candidate.title = theirs.to_owned();
            assert_eq!(
                check(&ours, &candidate, 3.0).verdict,
                Verdict::Accepted,
                "{theirs}"
            );
        }
    }

    #[test]
    fn a_remaster_written_with_a_dash_or_in_brackets_is_held_not_rejected() {
        let mut ours = track();
        ours.mix_name = String::new();
        for theirs in [
            "Night Signal - 2019 Remaster",
            "Night Signal (Remastered 2019)",
            "Night Signal \u{2013} Remastered",
        ] {
            let mut candidate = ours.clone();
            candidate.title = theirs.to_owned();
            let checked = check(&ours, &candidate, 3.0);
            assert_eq!(
                checked.verdict,
                Verdict::Held,
                "{theirs}: {}",
                checked.reason()
            );
            assert!(
                checked.reason().contains("remaster"),
                "{theirs}: {}",
                checked.reason()
            );
        }
        // The same remaster on both sides is the same recording.
        let mut both = ours.clone();
        both.title = "Night Signal - 2019 Remaster".to_owned();
        let mut candidate = ours.clone();
        candidate.title = "Night Signal (2019 Remaster)".to_owned();
        assert_eq!(check(&both, &candidate, 3.0).verdict, Verdict::Accepted);
        // A remaster of another title is still another title.
        candidate.title = "Day Signal - 2019 Remaster".to_owned();
        assert_eq!(check(&ours, &candidate, 3.0).verdict, Verdict::Rejected);
    }

    #[test]
    fn a_mix_written_after_a_dash_is_still_a_mix_and_a_real_difference_still_counts() {
        let mut dashed = track();
        dashed.title = "Night Signal - Extended Mix".to_owned();
        dashed.mix_name = String::new();
        assert_eq!(verdict(&dashed), Verdict::Accepted);
        dashed.title = "Night Signal - Radio Edit".to_owned();
        assert_eq!(verdict(&dashed), Verdict::Held);
        dashed.title = "Night Signal (VIP Remix)".to_owned();
        assert_eq!(verdict(&dashed), Verdict::Held);
        // A dash that is part of the title is left alone.
        let mut ours = track();
        ours.title = "Signal - Part Two".to_owned();
        let mut other = ours.clone();
        other.title = "Signal".to_owned();
        assert_eq!(check(&ours, &other, 3.0).verdict, Verdict::Rejected);
    }

    #[test]
    fn credits_split_into_names() {
        assert_eq!(
            artists("Kessler feat. Vane & Ode, Lark x Mira"),
            ["Kessler", "Vane", "Ode", "Lark", "Mira"]
        );
        assert_eq!(artists(""), Vec::<String>::new());
    }

    #[test]
    fn durations_are_read_as_seconds_or_clock_text() {
        assert_eq!(duration_of(&serde_json::json!(372)), Some(372.0));
        assert_eq!(duration_of(&serde_json::json!("6:12")), Some(372.0));
        assert_eq!(duration_of(&serde_json::json!("1:00:05")), Some(3605.0));
        assert_eq!(duration_of(&serde_json::json!("6:75")), None);
        assert_eq!(duration_of(&serde_json::json!("long")), None);
    }
}
