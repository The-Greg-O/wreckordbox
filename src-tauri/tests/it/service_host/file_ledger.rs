use super::*;

#[test]
fn phrase_publication_notifies_the_window_once_and_the_cli_sees_its_ledger_entry() {
    let window = window();
    let track = track_id(1);
    let relative = "/PIONEER/USBANLZ/phrase-ledger/ANLZ0000.DAT";
    fixture::set_analysis_path(&window.location, 1, relative).unwrap();
    let ext = rbl_anlz::sibling(
        &rbl_anlz::resolve(&window.location.share_root, relative),
        "EXT",
    );
    std::fs::create_dir_all(ext.parent().unwrap()).unwrap();
    let mut builder = rbl_anlz::AnlzBuilder::new();
    builder.path("/fixture.wav");
    std::fs::write(&ext, builder.finish()).unwrap();
    let mut file = rbl_anlz::Anlz::read(&ext).unwrap();
    let mut header = vec![0; 20];
    header[..4].copy_from_slice(&24_u32.to_be_bytes());
    header[4..6].copy_from_slice(&1_u16.to_be_bytes());
    header[6..8].copy_from_slice(&2_u16.to_be_bytes());
    header[14..16].copy_from_slice(&64_u16.to_be_bytes());
    let mut phrase = vec![0; 24];
    phrase[..2].copy_from_slice(&1_u16.to_be_bytes());
    phrase[2..4].copy_from_slice(&1_u16.to_be_bytes());
    phrase[4..6].copy_from_slice(&1_u16.to_be_bytes());
    file.sections
        .push(rbl_anlz::Section::new(b"PSSI", header, phrase));
    let before = file.to_bytes();
    std::fs::write(&ext, &before).unwrap();
    window.load();
    window.host().unwrap();
    let heard: Heard<String> = Arc::default();
    let captured = heard.clone();
    window.app.listen("analysis:changed", move |event| {
        captured
            .lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let mut client = window.client();
    client.subscribe().unwrap();
    let generation = window.generation();

    window.protect(true);
    let refused = run(rbxport_lib::analysis::edit_phrase(
        window.app.handle().clone(),
        window.state(),
        track.clone(),
        25,
        "cut".into(),
    ))
    .unwrap_err();
    assert_eq!(refused.message, rbl_library::protection::MESSAGE);
    assert_eq!(std::fs::read(&ext).unwrap(), before);
    assert_eq!(window.cli(&["--json", "runs", "list"]).json()["total"], 0);
    assert!(heard.lock().unwrap().is_empty());
    window.protect(false);
    assert!(run(rbxport_lib::analysis::edit_phrase(
        window.app.handle().clone(),
        window.state(),
        track.clone(),
        25,
        "cut".into()
    ))
    .unwrap());
    assert_ne!(std::fs::read(&ext).unwrap(), before);
    assert_eq!(*heard.lock().unwrap(), vec![track.clone()]);
    assert_eq!(window.generation(), generation);
    for protected in [true, false] {
        let event = client.next_event(Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(event.event, "protection_changed");
        assert_eq!(event.data["protect_library"], protected);
    }
    assert!(client
        .next_event(Duration::from_millis(100))
        .unwrap()
        .is_none());
    let runs = window.cli(&["--json", "runs", "list"]).json();
    assert_eq!(runs["total"], 1);
    let entries = ledger_entries(&window, &runs["runs"][0]["id"]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "phrase_cut");
    assert_eq!(entries[0]["command"]["args"]["track"], track);
    // Repeating the same cut is a no-op: no second notification or entry.
    assert!(!run(rbxport_lib::analysis::edit_phrase(
        window.app.handle().clone(),
        window.state(),
        track,
        25,
        "cut".into()
    ))
    .unwrap());
    assert_eq!(heard.lock().unwrap().len(), 1);
    assert_eq!(ledger_entries(&window, &runs["runs"][0]["id"]).len(), 1);
}
