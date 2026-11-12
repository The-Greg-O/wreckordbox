use super::*;

#[test]
fn a_bulk_scope_preserves_the_replacement_imports_transactional_mutant_follow() {
    let _turn = turn();
    let hosted = hosted();
    let entries = hosted.entries(&hosted.playlist);
    let original = hosted.mutate(&entries[0], json!({"key_shift": 2}));
    let edits = hosted.edits_of(original);
    let operation =
        rbl_library::bulk::Operation::new(&hosted.core, rbl_library::bulk::Kind::Import);
    operation
        .run(
            json!({"playlist":hosted.playlist}),
            || {
                window_replaces(
                    &hosted,
                    vec![hosted.tracks[1].clone(), hosted.tracks[0].clone()],
                );
                Ok::<_, rbl_db::DbError>(())
            },
            |_| Vec::new(),
        )
        .unwrap();
    assert_eq!(operation.finish(), None);
    let live = live_ids(&hosted);
    assert_eq!(live.len(), 1);
    assert_ne!(live[0], original);
    assert_eq!(hosted.edits_of(live[0]), edits);
    assert_eq!(hosted.mutant(original)["mutant"]["removed"], true);
    let new_entries = hosted.entries(&hosted.playlist);
    assert_eq!(hosted.mutant(live[0])["mutant"]["entry_id"], new_entries[1]);
    hosted.undo();
    assert_eq!(live_ids(&hosted), vec![original]);
    hosted.redo();
    assert_eq!(live_ids(&hosted), live);
}
