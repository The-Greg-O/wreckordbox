//! A reorder by playlist entry (wbx-59) is stored in the Ledger with its
//! entries and their numbers, and comes back from the stored form whole.
#![allow(clippy::unwrap_used, clippy::panic)]

use rbl_library::entry_order::EntryReorder;
use rbl_library::history::LibraryEdit;
use serde_json::json;
use wreckordbox_service::ledger::stored;

#[test]
fn an_entry_reorder_is_stored_by_entry_and_restored_whole() {
    let reorder = EntryReorder::from_parts(
        "900001".into(),
        vec![
            ("entry-a".into(), 1),
            ("entry-b".into(), 2),
            ("entry-c".into(), 3),
        ],
        vec![
            ("entry-c".into(), 1),
            ("entry-a".into(), 2),
            ("entry-b".into(), 3),
        ],
    );
    let edit = LibraryEdit::ReorderEntries(reorder.clone());
    assert_eq!(stored::kind_of(&edit), "reorder_playlist_entries");
    let payload = stored::store(&edit).unwrap();
    assert_eq!(
        payload,
        json!({
            "playlist": "900001",
            "before": [{"entry": "entry-a", "number": 1}, {"entry": "entry-b", "number": 2}, {"entry": "entry-c", "number": 3}],
            "after": [{"entry": "entry-c", "number": 1}, {"entry": "entry-a", "number": 2}, {"entry": "entry-b", "number": 3}],
        })
    );
    match stored::restore("reorder_playlist_entries", stored::FORMAT, &payload) {
        Some(LibraryEdit::ReorderEntries(back)) => assert_eq!(back, reorder),
        _ => panic!("not restored as an entry reorder"),
    }
}
