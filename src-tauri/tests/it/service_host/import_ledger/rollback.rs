//! A failed native import must remove its copies even if recording cannot end.
use super::*;
use rbl_library::{bulk, ledger};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

fn has_file(path: &Path) -> bool {
    std::fs::read_dir(path).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.is_file() || (path.is_dir() && has_file(&path))
    })
}

struct RefusingLedger {
    destination: PathBuf,
    ended_after_cleanup: Arc<AtomicBool>,
}
impl ledger::Ledger for RefusingLedger {
    fn begin(&self) -> Result<(), String> {
        assert!(
            has_file(&self.destination),
            "the command really copied the audio before admission"
        );
        Err("injected native admission failure".into())
    }
    fn written(&self, _: bool) {}
    fn record(
        &self,
        _: ledger::Step,
        _: &rbl_library::history::LibraryEdit,
        _: &str,
        _: ledger::Link,
    ) -> Option<i64> {
        None
    }
    fn note(&self, _: ledger::Unkept, _: Option<&str>) {}
    fn bulk(&self, _: bulk::Kind) -> Option<Arc<dyn bulk::Recorder>> {
        Some(Arc::new(FailingClose {
            destination: self.destination.clone(),
            ended_after_cleanup: self.ended_after_cleanup.clone(),
        }))
    }
}
struct FailingClose {
    destination: PathBuf,
    ended_after_cleanup: Arc<AtomicBool>,
}
impl bulk::Recorder for FailingClose {
    fn enter(&self, _: serde_json::Value) -> Box<dyn bulk::Step> {
        Box::new(NoRecords)
    }
    fn finish(&self) -> Option<String> {
        self.ended_after_cleanup
            .store(!has_file(&self.destination), Ordering::SeqCst);
        panic!("injected recording worker failure");
    }
}
struct NoRecords;
impl bulk::Step for NoRecords {
    fn complete(&mut self, _: Vec<serde_json::Value>) {}
    fn complete_generated(&mut self, _: &mut bulk::GenerateRecords<'_>) {}
}

#[test]
fn a_recording_worker_failure_cannot_skip_device_copy_rollback() {
    let window = window();
    window.load();
    let (stick, destination, playlist) = device_fixture(&window);
    let before = std::fs::read(stick.join("PIONEER/rekordbox/exportLibrary.db")).unwrap();
    let ended_after_cleanup = Arc::new(AtomicBool::new(false));
    window
        .state()
        .core()
        .history()
        .lock()
        .keep_ledger(Some(Arc::new(RefusingLedger {
            destination: destination.clone(),
            ended_after_cleanup: ended_after_cleanup.clone(),
        })));
    let result = run(rbxport_lib::device_library::device_playlist_import(
        window.app.handle().clone(),
        window.state(),
        stick.display().to_string(),
        "oneLibrary".into(),
        playlist.to_string(),
        rbxport_lib::device_library::MissingTracks::Copy,
        Some(destination.display().to_string()),
    ));
    assert!(result.is_err());
    assert!(
        !has_file(&destination),
        "a recorder failure must not strand the copied audio"
    );
    assert!(
        ended_after_cleanup.load(Ordering::SeqCst),
        "cleanup must precede recording shutdown"
    );
    assert_eq!(
        std::fs::read(stick.join("PIONEER/rekordbox/exportLibrary.db")).unwrap(),
        before
    );
}
