//! The DJ's source rules and the fixed order for one track and question.
//!
//! Rules are read afresh; resolving never updates a Fact. Supersession by
//! another source is a view of the current rules, distinct from a source
//! replacing its own earlier assertion in the stored history.
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Fact, Source, SourceKind};

/// The DJ's versionable file, alongside the Tag schema. Higher integers win;
/// unlisted sources have priority zero. The DJ cannot be demoted by a rule.
pub const RULES_FILE: &str = "source-precedence.toml";

/// Exact source identities, e.g. `[sources]` followed by `"agent:tagger" = 10`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)]
    sources: BTreeMap<String, i64>,
}

/// The first deciding comparison against the next best eligible Fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    NoEligibleFact,
    OnlyCandidate,
    Dj,
    SourcePrecedence,
    Confidence,
    LatestFact,
}

impl Reason {
    /// A short explanation suitable for CLI and window clients.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoEligibleFact => "No current Fact can stand for this question.",
            Self::OnlyCandidate => "This is the only current Fact for this question.",
            Self::Dj => "The DJ's judgement outranks every other source, regardless of confidence.",
            Self::SourcePrecedence => "The DJ's source rules give this source higher precedence.",
            Self::Confidence => "At equal source precedence, this Fact has higher confidence.",
            Self::LatestFact => {
                "Source precedence and confidence tie; the latest recorded Fact stands."
            }
        }
    }
}

impl Rules {
    /// Read the file without creating it or its directory. Missing means all
    /// non-DJ sources tie. Malformed or unreadable rules are an error.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(RULES_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        };
        let rules: Self = toml::from_str(&text)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?;
        for source in rules.sources.keys() {
            let valid = source.split_once(':').is_some_and(|(kind, name)| {
                matches!(kind, "agent" | "tool" | "web")
                    && !name.trim().is_empty()
                    && name == name.trim()
            });
            if !valid {
                return Err(format!("invalid {}: source {source:?} must be agent:NAME, tool:NAME or web:ADDRESS; the DJ always comes first", path.display()));
            }
        }
        Ok(rules)
    }

    /// The priority of this exact non-DJ source; zero unless listed.
    #[must_use]
    pub fn priority(&self, source: &Source) -> i64 {
        let kind = match source.kind {
            SourceKind::Dj => return 0,
            SourceKind::Agent => "agent",
            SourceKind::Tool => "tool",
            SourceKind::Web => "web",
        };
        self.sources
            .get(&format!("{kind}:{}", source.name))
            .copied()
            .unwrap_or(0)
    }

    /// Rank eligible Facts of ONE track and kind. The caller verifies
    /// Measurement fingerprints first. Archived/stale Facts cannot win.
    /// The durable insertion id breaks ties, independent of wall-clock jumps
    /// and input order. No store is changed, including supersession links.
    #[must_use]
    pub fn standing<'a>(&self, facts: &'a [Fact]) -> (Option<&'a Fact>, Reason) {
        let mut eligible: Vec<_> = facts
            .iter()
            .filter(|fact| fact.is_live() && !fact.is_stale())
            .collect();
        eligible.sort_by(|a, b| self.compare(b, a));
        let Some(first) = eligible.first().copied() else {
            return (None, Reason::NoEligibleFact);
        };
        let reason = eligible.get(1).map_or(Reason::OnlyCandidate, |second| {
            if first.source.kind == SourceKind::Dj && second.source.kind != SourceKind::Dj {
                Reason::Dj
            } else if self.priority(&first.source) != self.priority(&second.source) {
                Reason::SourcePrecedence
            } else if first.confidence != second.confidence {
                Reason::Confidence
            } else {
                Reason::LatestFact
            }
        });
        (Some(first), reason)
    }

    fn compare(&self, a: &Fact, b: &Fact) -> Ordering {
        (a.source.kind == SourceKind::Dj)
            .cmp(&(b.source.kind == SourceKind::Dj))
            .then_with(|| self.priority(&a.source).cmp(&self.priority(&b.source)))
            .then_with(|| a.confidence.total_cmp(&b.confidence))
            .then_with(|| a.id.cmp(&b.id))
    }
}
