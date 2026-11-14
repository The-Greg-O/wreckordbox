use super::*;
use serde_json::json;

fn xml(window: &Window, name: &str) -> String {
    let audio = window.dir.path().join(format!("{name}.wav"));
    super::bulk_ledger::wav(&audio);
    let path = window.dir.path().join(format!("{name}.xml"));
    std::fs::write(&path, format!(r#"<DJ_PLAYLISTS><COLLECTION><TRACK TrackID="1" Name="{name}" Location="file://localhost{}" Rating="153" Comments="Imported comment"><POSITION_MARK Name="Imported cue" Type="0" Start="1.0" Num="0"/></TRACK></COLLECTION><PLAYLISTS><NODE Type="0" Name="ROOT"><NODE Type="1" Name="Imported list" KeyType="0"><TRACK Key="1"/></NODE></NODE></PLAYLISTS></DJ_PLAYLISTS>"#, audio.display())).unwrap();
    path.display().to_string()
}

#[test]
fn xml_imports_are_one_run_each_including_replacements_without_dj_facts() {
    let window = window();
    window.load();
    window.host().unwrap();
    let mut observer = window.client();
    observer.subscribe().unwrap();
    let mut imported_tracks = Vec::new();
    for (index, name) in ["first", "replacement"].into_iter().enumerate() {
        let path = xml(&window, name);
        let report = run(commands::import_xml(
            window.app.handle().clone(),
            window.state(),
            path,
            Some(index > 0),
        ))
        .unwrap();
        assert_eq!(report.imported, 1);
        let track = &report.tracks[0].id;
        imported_tracks.push(track.clone());
        assert_eq!(window.rating(track), 3);
        let runs = window.cli(&["--json", "runs", "list"]).json();
        assert_eq!(runs["total"], index + 1, "{runs}");
        let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
        assert!(
            entries.iter().any(|e| e["kind"] == "import"
                && e["payload"]["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["track"] == *track)),
            "{entries:?}"
        );
        assert!(entries
            .iter()
            .all(|e| e["source"] == json!({"kind":"dj","name":"dj"})));
        let standing = window
            .cli(&["--json", "facts", "standing", track, "energy"])
            .json();
        assert!(
            standing["standing"].is_null(),
            "imported ratings are not hand assertions: {standing}"
        );
        assert_eq!(
            observer
                .next_event(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .event,
            "library_changed"
        );
        assert!(observer
            .next_event(Duration::from_millis(100))
            .unwrap()
            .is_none());
    }
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let playlist: String = db
        .connection()
        .query_row(
            "SELECT ID FROM djmdPlaylist WHERE Name='Imported list'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let contents = || {
        let page = window
            .cli(&["--json", "playlists", "tracks", &playlist])
            .json();
        page["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(contents(), vec![imported_tracks[1].clone()]);
    run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(contents(), vec![imported_tracks[0].clone()]);
    run(commands::redo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    assert_eq!(contents(), vec![imported_tracks[1].clone()]);
    let before = window.cli(&["--json", "runs", "list"]).json()["total"].clone();
    window.protect(true);
    let path = xml(&window, "refused");
    assert!(run(commands::import_xml(
        window.app.handle().clone(),
        window.state(),
        path,
        Some(true)
    ))
    .is_err());
    assert_eq!(
        window.cli(&["--json", "runs", "list"]).json()["total"],
        before
    );
}

#[test]
fn itunes_and_folder_imports_name_the_new_tracks_in_their_own_runs() {
    let window = window();
    window.load();
    window.host().unwrap();
    let audio = window.dir.path().join("itunes.wav");
    super::bulk_ledger::wav(&audio);
    let plist = window.dir.path().join("itunes.xml");
    std::fs::write(&plist, format!(r#"<plist version="1.0"><dict><key>Tracks</key><dict><key>1</key><dict><key>Track ID</key><integer>1</integer><key>Name</key><string>iTunes import</string><key>Location</key><string>file://{}</string><key>Rating</key><integer>80</integer></dict></dict><key>Playlists</key><array><dict><key>Name</key><string>iTunes list</string><key>Playlist Persistent ID</key><string>P1</string><key>Playlist Items</key><array><dict><key>Track ID</key><integer>1</integer></dict></array></dict></array></dict></plist>"#, audio.display())).unwrap();
    let imported = run(commands::import_itunes(
        window.app.handle().clone(),
        window.state(),
        plist.display().to_string(),
        None,
    ))
    .unwrap();
    assert_eq!(imported.imported, 1);
    let folder = window.dir.path().join("Folder import");
    std::fs::create_dir(&folder).unwrap();
    super::bulk_ledger::wav(&folder.join("folder.wav"));
    let dropped = run(commands::import_folder_playlist(
        window.app.handle().clone(),
        window.state(),
        folder.display().to_string(),
        "root".into(),
        None,
        None,
        None,
    ))
    .unwrap();
    assert_eq!(dropped.imported, 1);
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 2, "{runs}");
    for (run, track) in runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .zip([&dropped.tracks[0].id, &imported.tracks[0].id])
    {
        let entries = ledger_entries(&window, &run["id"]);
        assert!(
            entries.iter().any(|e| e["kind"] == "import"
                && e["payload"]["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["track"] == *track)),
            "{entries:?}"
        );
        assert!(window
            .cli(&["--json", "facts", "standing", track, "energy"])
            .json()["standing"]
            .is_null());
    }
}

fn device_fixture(window: &Window) -> (PathBuf, PathBuf, u32) {
    let stick = window.dir.path().join("fixture-stick");
    let destination = window.dir.path().join("imported-audio");
    std::fs::create_dir(&stick).unwrap();
    std::fs::create_dir(&destination).unwrap();
    let audio = window.dir.path().join("device.wav");
    super::bulk_ledger::wav(&audio);
    let mut dat = rbl_anlz::write::AnlzBuilder::new();
    dat.path("/device.wav").beat_grid(&[rbl_anlz::Beat {
        beat_number: 1,
        tempo_x100: 12700,
        time_ms: 25,
    }]);
    let tracks = [rbl_export::SourceTrack {
        id: 9999,
        source_path: audio,
        title: "Device import".into(),
        bpm_x100: 12700,
        key: "Fm".into(),
        analysis: vec![("DAT".into(), dat.finish())],
        ..Default::default()
    }];
    let playlists = [rbl_export::SourcePlaylist {
        id: 11,
        name: "Imported device list".into(),
        track_indices: vec![0],
        ..Default::default()
    }];
    rbl_export::export(&stick, &tracks, &playlists).unwrap();
    let library =
        rbl_export::device_library::read(&stick, rbl_export::device_library::Format::OneLibrary)
            .unwrap();
    let playlist = library
        .nodes
        .iter()
        .find(|n| n.name == "Imported device list")
        .unwrap()
        .id;
    (stick, destination, playlist)
}

#[test]
fn device_import_records_copied_tracks_and_metadata_without_writing_the_stick() {
    let window = window();
    window.load();
    window.host().unwrap();
    let (stick, destination, playlist) = device_fixture(&window);
    let before = std::fs::read(stick.join("PIONEER/rekordbox/exportLibrary.db")).unwrap();
    let report = run(rbxport_lib::device_library::device_playlist_import(
        window.app.handle().clone(),
        window.state(),
        stick.display().to_string(),
        "oneLibrary".into(),
        playlist.to_string(),
        rbxport_lib::device_library::MissingTracks::Copy,
        Some(destination.display().to_string()),
    ))
    .unwrap();
    assert_eq!(
        (report.copied, report.tracks, report.analysis_failed),
        (1, 1, 0)
    );
    assert_eq!(
        std::fs::read(stick.join("PIONEER/rekordbox/exportLibrary.db")).unwrap(),
        before
    );
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 1, "{runs}");
    let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
    assert!(
        entries
            .iter()
            .all(|e| e["source"] == json!({"kind":"dj","name":"dj"})),
        "{entries:?}"
    );
    let records: Vec<_> = entries
        .iter()
        .flat_map(|e| e["payload"]["records"].as_array().into_iter().flatten())
        .collect();
    assert!(
        records
            .iter()
            .any(|r| r["playlist"] == report.id.as_deref().unwrap()),
        "{entries:?}"
    );
    assert!(
        records
            .iter()
            .any(|r| r["operation"] == "device_analysis" && r["track"].is_string()),
        "{entries:?}"
    );
    let track = records
        .iter()
        .find(|r| r["operation"] == "device_analysis")
        .unwrap()["track"]
        .as_str()
        .unwrap();
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let relative: String = db
        .connection()
        .query_row(
            "SELECT AnalysisDataPath FROM djmdContent WHERE ID=?1",
            [track],
            |row| row.get(0),
        )
        .unwrap();
    let copied =
        rbl_anlz::Anlz::read(&rbl_anlz::resolve(&window.location.share_root, &relative)).unwrap();
    assert_eq!(copied.beat_grid().unwrap()[0].tempo_x100, 12700);
    assert!(window
        .cli(&["--json", "facts", "standing", track, "energy"])
        .json()["standing"]
        .is_null());
}

#[test]
fn an_import_append_failure_preserves_accepted_rows_and_announces_a_warning() {
    let window = window();
    window.load();
    window.host().unwrap();
    let sidecar_dir = window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar");
    drop(wreckordbox_sidecar::Sidecar::open(&sidecar_dir).unwrap());
    let db = rusqlite::Connection::open(sidecar_dir.join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_ledger BEFORE INSERT ON ledger BEGIN SELECT RAISE(ABORT, 'fixture append failure'); END;").unwrap();
    let warnings: Arc<Mutex<Vec<String>>> = Arc::default();
    let heard = warnings.clone();
    window.app.listen("ledger:recording-warning", move |event| {
        heard
            .lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let path = xml(&window, "accepted-before-recording-failed");
    let report = run(commands::import_xml(
        window.app.handle().clone(),
        window.state(),
        path,
        None,
    ))
    .unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(window.rating(&report.tracks[0].id), 3);
    assert_eq!(
        warnings.lock().unwrap().len(),
        1,
        "recording failure must reach the window"
    );
    let runs = window.cli(&["--json", "runs", "list"]).json();
    let args = ["--json", "runs", "show", &runs["runs"][0]["id"].to_string()];
    let pending = window.cli(&args).json();
    assert!(pending["pending_recording"]["total"].as_u64().unwrap() > 0);
    assert_eq!(window.cli(&args).json(), pending, "reading must not replay");
    db.execute_batch("DROP TRIGGER refuse_ledger").unwrap();
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![track_id(0)],
        4,
    ))
    .unwrap();
    let recorded = window.cli(&args).json();
    assert_eq!(recorded["pending_recording"]["total"], 0);
    assert!(recorded["entries"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["payload"]["records"].as_array().into_iter().flatten())
        .any(|r| r["track"] == report.tracks[0].id));
}

#[test]
fn a_replacement_history_append_failure_is_reported_even_when_bulk_results_succeed() {
    let window = window();
    window.load();
    window.host().unwrap();
    run(commands::import_xml(
        window.app.handle().clone(),
        window.state(),
        xml(&window, "before-history-failure"),
        None,
    ))
    .unwrap();
    let dir = window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar");
    let db = rusqlite::Connection::open(dir.join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_replacement BEFORE INSERT ON ledger WHEN NEW.kind = 'replace_playlist_contents' BEGIN SELECT RAISE(ABORT, 'fixture history append failure'); END;").unwrap();
    let report = run(commands::import_xml(
        window.app.handle().clone(),
        window.state(),
        xml(&window, "after-history-failure"),
        Some(true),
    ))
    .unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(window.rating(&report.tracks[0].id), 3);
    assert!(
        report.recording_warning.is_some(),
        "kept replacement history is part of the import's recording"
    );
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 2);
    let id = runs["runs"][0]["id"].clone();
    assert!(ledger_entries(&window, &id)
        .iter()
        .any(|e| e["kind"] == "import"));
    db.execute_batch("DROP TRIGGER refuse_replacement").unwrap();
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![track_id(0)],
        4,
    ))
    .unwrap();
    assert!(ledger_entries(&window, &id)
        .iter()
        .any(|e| e["kind"] == "replace_playlist_contents"));
}

#[path = "import_ledger/rollback.rs"]
mod rollback;
