//! File-tag spelling rules shared with the interface's keySpelling.ts.
//! Keep the cross-language keySpellings.fixture.tsv passing when extending them.

fn boundary(rest: &str) -> bool {
    rest.chars().next().is_none_or(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '/' | ',' | ':' | ';' | '(' | ')' | '[' | ']' | '–' | '—' | '-'
            )
    })
}

pub(super) fn rank(raw: &str) -> Option<u32> {
    let text = raw.trim();
    let first = text.chars().next()?;
    if first.is_ascii_digit() {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 2 {
            return None;
        }
        let number = text[..digits].parse::<u32>().ok()?;
        if !(1..=12).contains(&number) {
            return None;
        }
        let mode = text[digits..].chars().next()?;
        if !boundary(&text[digits + mode.len_utf8()..]) {
            return None;
        }
        return match mode.to_ascii_lowercase() {
            'a' => Some((number - 1) * 2),
            'b' => Some((number - 1) * 2 + 1),
            'm' => Some(((number + 6) % 12) * 2),
            'd' => Some(((number + 6) % 12) * 2 + 1),
            _ => None,
        };
    }
    let mut pitch: i32 = match first.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let mut rest = &text[first.len_utf8()..];
    if let Some(accidental) = rest.chars().next() {
        match accidental {
            '#' | '♯' => pitch += 1,
            'b' | 'B' | '♭' => pitch -= 1,
            _ => {}
        }
        if matches!(accidental, '#' | '♯' | 'b' | 'B' | '♭') {
            rest = &rest[accidental.len_utf8()..];
        }
    }
    let suffix = rest.trim_start();
    let mode = ["minor", "major", "min", "maj", "m"]
        .into_iter()
        .find(|word| {
            suffix
                .get(..word.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(word))
                && boundary(&suffix[word.len()..])
        });
    if mode.is_none() && !boundary(rest) {
        return None;
    }
    let minor = matches!(mode, Some("minor" | "min" | "m"));
    let pitch = usize::try_from(pitch.rem_euclid(12)).ok()?;
    let wheel = if minor {
        [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10]
    } else {
        [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1]
    };
    Some((wheel[pitch] - 1) * 2 + u32::from(!minor))
}
