//! A step of the edit history that changes no Library row: the DJ's own
//! judgements (Facts) of kinds the Library cannot hold, such as a Tag
//! schema dimension that does not reach the deck (wbx-65).
//!
//! It is an entry like any other so that Undo reverses the DJ's last
//! action whichever store it touched: an Energy edit, then a tag kept as a
//! Fact, then Undo takes back the tag, and a second Undo the Energy. The
//! core writes nothing for it; the history's Ledger (the Library service)
//! records the Facts from the entry's payload when it is recorded,
//! withdraws them on Undo and records them again on Redo, exactly as it
//! does for the Facts it derives from a native edit (see the service's
//! `dj_facts`).

/// One kind of judgement on one track, before and after, each as the JSON
/// text of the Fact's value (`null`: no opinion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementChange {
    pub track: String,
    pub kind: String,
    pub before: String,
    pub after: String,
}

/// The judgements one action changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Judgements {
    pub changes: Vec<JudgementChange>,
}

impl Judgements {
    /// Nothing would change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes
            .iter()
            .all(|change| change.before == change.after)
    }

    /// The tracks it names, each once.
    #[must_use]
    pub fn tracks(&self) -> Vec<String> {
        let mut tracks: Vec<String> = Vec::new();
        for change in &self.changes {
            if !tracks.contains(&change.track) {
                tracks.push(change.track.clone());
            }
        }
        tracks
    }
}
