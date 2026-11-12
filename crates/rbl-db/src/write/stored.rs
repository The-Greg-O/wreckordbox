//! The exact values an edit's undo payload holds, read out and put back.
//!
//! wreckordbox's Ledger (ADR 0004) keeps the before-and-after payloads the
//! edit history captures in a durable form of its own, and an Unwind later
//! rebuilds a payload from that form to invert it through the writer's own
//! undo calls. The payloads keep their fields private, so this module, a
//! child of the one that defines them, is the one place that reads them out
//! whole (`parts`) and builds them again (`from_parts`). Nothing here
//! writes, and no payload changes shape.
//!
//! Covered: [`TrackEdit`], [`TrackTagEdit`] and [`MyTagOrder`], the
//! payloads of every edit the Library service's commands make today. The
//! other payloads are added here the same way when the Ledger stores them.
//!
//! Read only (`parts`, no `from_parts`): the playlist membership payloads
//! [`PlaylistTrackRemoval`], [`PlaylistTrackAddition`] and
//! [`MembershipReplacement`], whose entry rows a Mutant (keyed to a
//! `djmdSongPlaylist.ID`) follows when they leave a playlist or come back
//! (wbx-80).

use rusqlite::types::Value;

use super::{
    MembershipReplacement, MyTagOrder, PlaylistTrackAddition, PlaylistTrackRemoval, TrackEdit,
    TrackTagEdit, WRITABLE_COLUMNS,
};

/// One track column's value before and after an edit.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackEditParts {
    /// The track (`djmdContent.ID`).
    pub track: String,
    /// The `djmdContent` column.
    pub column: String,
    pub before: Value,
    pub after: Value,
}

/// One track's My Tags before and after an edit, as `djmdMyTag` ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackTagEditParts {
    pub track: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// One run of My Tag siblings before and after a reorder: each sibling's id
/// and exact `Seq`, in the order read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyTagOrderParts {
    /// [`super::ROOT`] for the categories, otherwise the category.
    pub parent: String,
    pub before: Vec<(String, Option<i64>)>,
    pub after: Vec<(String, Option<i64>)>,
}

impl TrackEdit {
    /// Everything the payload holds.
    #[must_use]
    pub fn parts(&self) -> TrackEditParts {
        TrackEditParts {
            track: self.content.clone(),
            column: self.column.to_owned(),
            before: self.before.clone(),
            after: self.after.clone(),
        }
    }

    /// The payload these parts were read from. `None` when the column is
    /// not one a track edit may write.
    #[must_use]
    pub fn from_parts(parts: TrackEditParts) -> Option<Self> {
        let column = WRITABLE_COLUMNS.iter().find(|known| **known == parts.column)?;
        Some(Self { content: parts.track, column, before: parts.before, after: parts.after })
    }
}

impl TrackTagEdit {
    /// Everything the payload holds.
    #[must_use]
    pub fn parts(&self) -> TrackTagEditParts {
        TrackTagEditParts { track: self.content.clone(), before: self.before.clone(), after: self.after.clone() }
    }

    /// The payload these parts were read from.
    #[must_use]
    pub fn from_parts(parts: TrackTagEditParts) -> Self {
        Self { content: parts.track, before: parts.before, after: parts.after }
    }
}

impl MyTagOrder {
    /// Everything the payload holds.
    #[must_use]
    pub fn parts(&self) -> MyTagOrderParts {
        MyTagOrderParts { parent: self.parent.clone(), before: self.before.clone(), after: self.after.clone() }
    }

    /// The payload these parts were read from.
    #[must_use]
    pub fn from_parts(parts: MyTagOrderParts) -> Self {
        Self { parent: parts.parent, before: parts.before, after: parts.after }
    }
}

/// A playlist's membership rows (`djmdSongPlaylist.ID`) an edit moved:
/// those it took out or put in, and the rows of the playlist around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipParts<'a> {
    /// The playlist (`djmdPlaylist.ID`).
    pub playlist: &'a str,
    /// For a removal, the rows it took out; for an addition, the rows it
    /// made; for a replacement, the rows the playlist held before.
    pub moved: Vec<&'a str>,
    /// For a removal, every row of the playlist before it, in order; for
    /// an addition, the rows after it; for a replacement, the rows the
    /// playlist holds after.
    pub around: Vec<&'a str>,
}

impl PlaylistTrackRemoval {
    /// The playlist, the rows taken out, and its rows before.
    #[must_use]
    pub fn parts(&self) -> MembershipParts<'_> {
        MembershipParts {
            playlist: &self.playlist,
            moved: self.removed.iter().map(String::as_str).collect(),
            around: self.order.iter().map(String::as_str).collect(),
        }
    }
}

impl PlaylistTrackAddition {
    /// The playlist, the rows made, and its rows after.
    #[must_use]
    pub fn parts(&self) -> MembershipParts<'_> {
        MembershipParts {
            playlist: &self.playlist,
            moved: self.added.iter().map(String::as_str).collect(),
            around: self.after.iter().map(|(id, _)| id.as_str()).collect(),
        }
    }
}

impl MembershipReplacement {
    /// The playlist, its rows before the replacement, and after.
    #[must_use]
    pub fn parts(&self) -> MembershipParts<'_> {
        MembershipParts {
            playlist: &self.playlist,
            moved: self.before.iter().map(String::as_str).collect(),
            around: self.after.iter().map(String::as_str).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_rebuilt_from_its_parts_is_the_payload() {
        let edit = TrackEdit { content: "10003".into(), column: "Rating", before: Value::Integer(2), after: Value::Null };
        assert_eq!(TrackEdit::from_parts(edit.parts()), Some(edit));

        let tags = TrackTagEdit { content: "10003".into(), before: vec!["7".into()], after: vec!["7".into(), "9".into()] };
        assert_eq!(TrackTagEdit::from_parts(tags.parts()), tags);

        let order = MyTagOrder {
            parent: super::super::ROOT.into(),
            before: vec![("1".into(), Some(1)), ("2".into(), None)],
            after: vec![("2".into(), Some(1)), ("1".into(), Some(2))],
        };
        assert_eq!(MyTagOrder::from_parts(order.parts()), order);
    }

    #[test]
    fn a_column_no_track_edit_writes_builds_no_payload() {
        let parts = TrackEditParts { track: "1".into(), column: "rb_local_usn".into(), before: Value::Null, after: Value::Null };
        assert_eq!(TrackEdit::from_parts(parts), None);
    }
}
