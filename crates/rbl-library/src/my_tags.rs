//! The My Tag edits that are more than one write: a tick on many tracks,
//! and the comments that follow a My Tag edit while rekordbox's Add "My
//! Tag" to the "Comments" is on (#605).
//!
//! Each is a closure body for [`crate::LibraryCore::edit`] (or the shell's
//! recorded edit), so the window and the Library service make the same
//! writes in the same order and put back the same things on a failure.

use rbl_db::write::{TrackEdit, TrackTagEdit, Writer};
use rbl_db::DbError;

/// A tick in the My Tag panel: `tag` goes on each of `tracks`, or comes
/// off. Every track or none: on a failure the tracks already changed are
/// put back and the error returned. A track no longer in the collection is
/// passed over, and one that already has the tag (or lacks it) yields no
/// edit.
pub fn put_on_tracks(
    w: &mut Writer,
    tracks: &[String],
    tag: &str,
    on: bool,
) -> Result<Vec<TrackTagEdit>, DbError> {
    let mut edits: Vec<TrackTagEdit> = Vec::with_capacity(tracks.len());
    for track in tracks {
        let step = match w.has_track(track) {
            Ok(false) => continue,
            Ok(true) => w.put_my_tag_with_undo(track, tag, on),
            Err(error) => Err(error),
        };
        match step {
            Ok((_, edit)) => {
                if !edit.is_empty() {
                    edits.push(edit);
                }
            }
            Err(error) => {
                undo_tag_edits(w, &edits);
                return Err(error);
            }
        }
    }
    Ok(edits)
}

/// Puts back `edits`, last first, after a failure part of the way through
/// an edit. One that cannot be put back is logged; the failure that led
/// here is the one the caller reports.
pub fn undo_tag_edits(w: &mut Writer, edits: &[TrackTagEdit]) {
    for applied in edits.iter().rev().filter(|edit| !edit.is_empty()) {
        if let Err(undo) = w.undo_tag_edit(applied) {
            tracing::error!(%undo, "a My Tag edit could not be put back after a failure");
        }
    }
}

/// Add "My Tag" to the "Comments" (#605): rewrites the comment of each of
/// `tracks` with its My Tags, as rekordbox's `BrowseListViewer::
/// updateCommentWithMyTag` does for every selected track after a My Tag
/// edit, whether or not that track's own tags changed. A track no longer in
/// the collection is passed over. On a failure the comments already
/// written and `tags` are put back, and the error returned.
pub fn refresh_comments(
    w: &mut Writer,
    tracks: &[String],
    tags: &[TrackTagEdit],
) -> Result<Vec<TrackEdit>, DbError> {
    let mut comments: Vec<TrackEdit> = Vec::with_capacity(tracks.len());
    for track in tracks {
        let step = match w.has_track(track) {
            Ok(false) => continue,
            Ok(true) => w.refresh_my_tag_comment_with_undo(track),
            Err(error) => Err(error),
        };
        match step {
            Ok((_, edit)) => {
                if !edit.is_empty() {
                    comments.push(edit);
                }
            }
            Err(error) => {
                for applied in comments.iter().rev() {
                    if let Err(undo) = w.undo_track_edit(applied) {
                        tracing::error!(%undo, "a My Tag comment could not be put back after a failure");
                    }
                }
                undo_tag_edits(w, tags);
                return Err(error);
            }
        }
    }
    Ok(comments)
}

/// The comment rewrite after a rename or a delete, which no history entry
/// holds: each track still in the collection, written as it comes.
pub fn rewrite_comments(w: &mut Writer, tracks: &[String]) -> Result<(), DbError> {
    for track in tracks {
        if w.has_track(track)? {
            w.refresh_my_tag_comment_with_undo(track)?;
        }
    }
    Ok(())
}
