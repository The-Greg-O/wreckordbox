use super::*;

#[test]
fn preparing_audio_history_does_not_hold_the_library_edit_gate() {
    use rbl_library::history::{LibraryEdit, OutsideSteps};
    use std::sync::Arc;
    struct Audio(std::sync::Weak<LibraryCore>);
    struct PreparedAudio;
    impl OutsideSteps for PreparedAudio {
        fn is_empty(&self) -> bool {
            false
        }
        fn check(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            Ok(())
        }
        fn step(
            &self,
            _: bool,
            _: &str,
        ) -> Result<rbl_library::history::OutsideHold, rbl_db::DbError> {
            Ok(Box::new(()))
        }
        fn revert(&self, _: rbl_library::history::OutsideHold, _: bool) -> (usize, usize) {
            (0, 1)
        }
    }
    impl OutsideSteps for Audio {
        fn prepare(
            &self,
            undo: bool,
            label: &str,
        ) -> Result<Option<Arc<dyn OutsideSteps>>, rbl_db::DbError> {
            self.check(undo, label)?;
            Ok(Some(Arc::new(PreparedAudio)))
        }
        fn is_empty(&self) -> bool {
            false
        }
        fn check(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            let core = self.0.upgrade().unwrap();
            assert!(
                std::thread::spawn(move || core.edit_gate().try_lock().is_some())
                    .join()
                    .unwrap(),
                "audio preparation must allow another Library edit"
            );
            Ok(())
        }
        fn step(
            &self,
            _: bool,
            _: &str,
        ) -> Result<rbl_library::history::OutsideHold, rbl_db::DbError> {
            Ok(Box::new(()))
        }
        fn revert(&self, _: rbl_library::history::OutsideHold, _: bool) -> (usize, usize) {
            (0, 1)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(core_in(dir.path()));
    let track = rbl_db::fixture::track_id(1);
    let audio = Arc::new(Audio(Arc::downgrade(&core)));
    let stars = if rating_of(&core, &track) == 4 { 3 } else { 4 };
    core.recorded_edit(&NoHost, Touched::Tracks, "Audio key", |w| {
        let (_, edit) = w.set_rating_with_undo(&track, stars)?;
        Ok((
            (),
            LibraryEdit::KeyEdit {
                tracks: vec![edit],
                files: audio,
            },
        ))
    })
    .unwrap();
    core.step_history(&NoHost, true).unwrap();
}

#[test]
fn a_protection_turn_or_new_history_entry_during_audio_preparation_is_preserved() {
    use rbl_library::history::{LibraryEdit, OutsideSteps};
    use std::sync::Arc;
    struct During {
        core: std::sync::Weak<LibraryCore>,
        protect: bool,
        published: Arc<AtomicUsize>,
    }
    impl OutsideSteps for During {
        fn is_empty(&self) -> bool {
            false
        }
        fn prepare(
            &self,
            _: bool,
            _: &str,
        ) -> Result<Option<Arc<dyn OutsideSteps>>, rbl_db::DbError> {
            let core = self.core.upgrade().unwrap();
            if self.protect {
                core.set_protection(true).unwrap();
            } else {
                let other = rbl_db::fixture::track_id(2);
                let stars = if rating_of(&core, &other) == 5 { 4 } else { 5 };
                core.recorded_edit(&NoHost, Touched::Tracks, "Later edit", |w| {
                    let (_, edit) = w.set_rating_with_undo(&other, stars)?;
                    Ok(((), LibraryEdit::Track(vec![edit])))
                })
                .unwrap();
            }
            Ok(None)
        }
        fn check(&self, _: bool, _: &str) -> Result<(), rbl_db::DbError> {
            Ok(())
        }
        fn step(
            &self,
            _: bool,
            _: &str,
        ) -> Result<rbl_library::history::OutsideHold, rbl_db::DbError> {
            self.published.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(()))
        }
        fn revert(&self, _: rbl_library::history::OutsideHold, _: bool) -> (usize, usize) {
            (0, 1)
        }
    }
    for protect in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(core_in(dir.path()));
        let track = rbl_db::fixture::track_id(1);
        let published = Arc::new(AtomicUsize::new(0));
        let stars = if rating_of(&core, &track) == 4 { 3 } else { 4 };
        core.recorded_edit(&NoHost, Touched::Tracks, "Audio edit", |w| {
            let (_, edit) = w.set_rating_with_undo(&track, stars)?;
            Ok((
                (),
                LibraryEdit::KeyEdit {
                    tracks: vec![edit],
                    files: Arc::new(During {
                        core: Arc::downgrade(&core),
                        protect,
                        published: published.clone(),
                    }),
                },
            ))
        })
        .unwrap();
        assert!(core.step_history(&NoHost, true).is_err());
        assert_eq!(published.load(Ordering::SeqCst), 0);
        assert_eq!(rating_of(&core, &track), stars);
        assert_eq!(
            core.history().lock().undo_label(),
            Some(if protect { "Audio edit" } else { "Later edit" })
        );
        if !protect {
            core.step_history(&NoHost, true).unwrap();
            assert_eq!(core.history().lock().undo_label(), Some("Audio edit"));
        }
    }
}
