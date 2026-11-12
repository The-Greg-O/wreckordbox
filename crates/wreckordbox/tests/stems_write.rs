//! `wreckordbox stems write` against a Library service on a throwaway
//! Library: the real binary, its output, and what is on the Stick
//! afterwards, read by the independent reader of the Native stems layout
//! (`deck`, shared with the writer crate's own tests, sharing no code with
//! the writer). The Stick is always a temporary folder, the Stems small
//! synthetic WAV files recorded straight into a fixture Sidecar. No
//! separation is run.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;
#[path = "../../wreckordbox-native-stems/tests/deck/mod.rs"]
mod deck;

use std::path::{Path, PathBuf};

use common::Sandbox;
use deck::Identity;
use rbl_export::device_library::{self, Format};
use wreckordbox_sidecar::stems::{NewStems, Role};
use wreckordbox_sidecar::{Sidecar, Source, SourceKind};

const MARKER: &str = ".rbxport-disposable";
/// Long enough that an encoded Stem spans several Ogg pages, as real ones
/// do: a decoder trims a stream to its exact length only then.
const FRAMES: u32 = 44_100;

/// A 16-bit stereo 44.1 kHz WAV of `frames` sample frames: silence, then
/// from a fifth of the way in a tone of `hertz` with noise over it.
fn write_audio(path: &Path, frames: u32, hertz: f64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let mut noise = 0x2545_f491_u32;
    for frame in 0..frames {
        let sample = if frame < frames / 5 {
            0
        } else {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let t = f64::from(frame) / 44_100.0;
            ((t * hertz * std::f64::consts::TAU).sin() * 16_000.0
                + f64::from((noise >> 16) as i16) / 8.0) as i16
        };
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// A Library track with an audio file, a Stick, and the two together.
struct Rig {
    sandbox: Sandbox,
    stick: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        Self {
            sandbox: Sandbox::new(),
            stick: tempfile::tempdir().unwrap(),
        }
    }

    fn stick(&self) -> &Path {
        self.stick.path()
    }

    fn audio(&self, name: &str) -> PathBuf {
        self.sandbox.path().join("audio").join(name)
    }

    /// Adds a track to the Library as rekordbox imports one. Before any
    /// service starts.
    fn import(&self, name: &str) -> String {
        let path = self.audio(name);
        write_audio(&path, FRAMES, 440.0);
        let mut id = String::new();
        self.sandbox
            .edit(|writer| id = writer.import_file(&path).unwrap());
        id
    }

    /// Records four synthetic Stems of `frames` sample frames for the
    /// track in the fixture Sidecar, through the Sidecar's own interface,
    /// as made from the audio file the track has now. Before any service
    /// starts (one process owns the Sidecar).
    fn record_stems(&self, track: &str, name: &str, frames: u32) {
        let sidecar = Sidecar::open(&self.sandbox.sidecar_dir()).unwrap();
        let source = Source::new(SourceKind::Tool, "separation-engine");
        let run = sidecar
            .begin_run(&source, "Synthetic Stems for a test")
            .unwrap();
        let workspace = sidecar.workspace(run).unwrap();
        let files: Vec<(Role, PathBuf)> = Role::ALL
            .into_iter()
            .zip([880.0, 660.0, 110.0, 220.0])
            .map(|(role, hertz)| {
                let path = workspace.join(format!("{role}.wav"));
                write_audio(&path, frames, hertz);
                (role, path)
            })
            .collect();
        let fingerprint = wreckordbox_sidecar::fingerprint_file(&self.audio(name)).unwrap();
        sidecar
            .record_stems(&NewStems {
                track_id: track,
                fingerprint: &fingerprint,
                parent_samples: u64::from(frames),
                sample_rate: 44_100,
                files: &files,
                source: &source,
                produced_by: serde_json::json!({"engine": "test"}),
                run,
            })
            .unwrap();
        sidecar.finish_run(run, "recorded").unwrap();
    }

    /// Exports the track to the Stick with the export pipeline.
    fn export(&self, track: &str, name: &str) {
        let source = rbl_export::SourceTrack {
            id: track.parse().unwrap(),
            source_path: self.audio(name),
            title: "Night Bus".into(),
            artist: "Artist".into(),
            album: "Single".into(),
            ..Default::default()
        };
        let playlist = rbl_export::SourcePlaylist {
            id: 10,
            name: "Set".into(),
            track_indices: vec![0],
            ..Default::default()
        };
        rbl_export::export(self.stick(), &[source], &[playlist]).unwrap();
    }

    fn mark_spare(&self) {
        std::fs::write(self.stick().join(MARKER), b"").unwrap();
    }

    /// A track in the Library with current Stems, exported to a Spare stick.
    fn ready(name: &str) -> (Self, String) {
        let rig = Self::new();
        let track = rig.import(name);
        rig.record_stems(&track, name, FRAMES);
        rig.export(&track, name);
        rig.mark_spare();
        (rig, track)
    }

    /// The track's file on the Stick, as the Stick's library spells it.
    fn track_file(&self, track: &str) -> PathBuf {
        let library = device_library::read(self.stick(), Format::OneLibrary).unwrap();
        let id: u64 = track.parse().unwrap();
        let held = library
            .tracks
            .iter()
            .find(|t| t.master_content_id == id)
            .unwrap();
        self.stick().join(held.path.trim_start_matches('/'))
    }

    fn write(&self, track: &str, json: bool) -> common::Run {
        let stick = self.stick().to_str().unwrap();
        if json {
            self.sandbox
                .cli(&["--json", "stems", "write", track, stick])
        } else {
            self.sandbox.cli(&["stems", "write", track, stick])
        }
    }
}

#[test]
fn native_stems_are_written_to_a_spare_stick_and_the_deck_plays_all_four() {
    let (rig, track) = Rig::ready("Night Bus.wav");

    let run = rig.write(&track, true);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let written = run.json();
    assert_eq!(written["track"]["id"], track.as_str());
    assert_eq!(written["track"]["title"], "Night Bus");
    assert_eq!(written["outcome"], "written");
    assert_eq!(written["stick"], rig.stick().to_str().unwrap());
    assert_eq!(
        written["track_path"],
        "Contents/Artist/Single/Night Bus.wav"
    );
    assert_eq!(written["key"], "Night Bus");
    assert_eq!(written["folder"], "STEMS/Night Bus");
    assert_eq!(
        written["files"],
        serde_json::json!(["vocals.ogg", "other.ogg", "bass.ogg", "drums.ogg"])
    );
    assert_eq!(written["samples"], FRAMES);
    assert_eq!(written["sample_rate"], 44_100);
    assert_eq!(written["replaced"], false);
    assert_eq!(written["skipped"], serde_json::Value::Null);

    // The deck, from the path its library holds for the track.
    let loaded =
        deck::load(&[rig.stick()], &rig.track_file(&track)).expect("the deck finds the set");
    assert_eq!(loaded.folder, rig.stick().join("STEMS/Night Bus"));
    assert_eq!((loaded.rank, loaded.identity), (0, Identity::Proven));
    assert_eq!(
        loaded.pads.keys().copied().collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    for (pad, stem) in &loaded.pads {
        assert_eq!((stem.sample_rate, stem.channels), (44_100, 2), "pad {pad}");
        // Exactly the parent's length, as the Sidecar holds it.
        assert_eq!(stem.frames, u64::from(FRAMES), "pad {pad}");
        // Aligned with the source: silent until the sound starts a fifth
        // of the way in, give or take one short Vorbis block.
        let onset = u64::from(FRAMES / 5);
        assert!(stem.peak(0, onset - 256) < 200, "pad {pad} starts early");
        assert!(
            stem.peak(onset, onset + 512) > 8_000,
            "pad {pad} starts late"
        );
    }
    assert!(deck::counts_as_made(&loaded.folder));
}

#[test]
fn the_sticks_exported_library_still_reads_back_and_only_the_stems_folder_was_added() {
    let (rig, track) = Rig::ready("Night Bus.wav");
    let libraries = |rig: &Rig| {
        [Format::DeviceLibrary, Format::OneLibrary]
            .map(|format| device_library::read(rig.stick(), format).unwrap())
    };
    let (before, files_before) = (libraries(&rig), deck::snapshot(rig.stick()));

    let run = rig.write(&track, false);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(libraries(&rig), before);
    assert_eq!(before[1].tracks.len(), 1);
    let report = rbl_export::verify(rig.stick()).unwrap();
    assert!(
        report.is_ok(),
        "{}",
        rbl_export::verification_failure(&report)
    );
    let files_after = deck::snapshot(rig.stick());
    let added: Vec<&PathBuf> = files_after
        .keys()
        .filter(|path| !files_before.contains_key(*path))
        .collect();
    assert_eq!(added.len(), 5, "{added:?}");
    assert!(
        added.iter().all(|path| path.starts_with("STEMS/Night Bus")),
        "{added:?}"
    );
    for (path, bytes) in &files_before {
        assert!(
            files_after.get(path) == Some(bytes),
            "{} changed",
            path.display()
        );
    }
}

#[test]
fn the_text_output_says_where_the_stems_went() {
    let (rig, track) = Rig::ready("Night Bus.wav");

    let run = rig.write(&track, false);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "Wrote Native stems for track {track} \"Night Bus\" to {}\n\
             Track on the Stick  Contents/Artist/Single/Night Bus.wav\n\
             Stems               vocals.ogg, other.ogg, bass.ogg, drums.ogg ({FRAMES} samples at 44100 Hz)\n",
            rig.stick().join("STEMS/Night Bus").display()
        )
    );

    // Writing them again replaces the set, and says so.
    let again = rig.write(&track, false);
    assert!(again.status.success(), "{}", again.stderr);
    assert!(
        again.stdout.starts_with(&format!(
            "Replaced the Native stems for track {track} \"Night Bus\" at "
        )),
        "{}",
        again.stdout
    );
}

#[test]
fn a_stick_without_the_spare_stick_marker_is_refused_and_not_written_to() {
    let rig = Rig::new();
    let track = rig.import("Night Bus.wav");
    rig.record_stems(&track, "Night Bus.wav", FRAMES);
    rig.export(&track, "Night Bus.wav");
    let before = deck::snapshot(rig.stick());

    let run = rig.write(&track, true);

    assert!(!run.status.success());
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "not_a_spare_stick");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("Spare stick"), "{message}");
    assert!(message.contains(MARKER), "{message}");
    assert_eq!(deck::snapshot(rig.stick()), before);
    assert!(!rig.stick().join("STEMS").exists());

    // Marked, the same command writes.
    rig.mark_spare();
    assert!(rig.write(&track, true).status.success());
}

#[test]
fn a_track_with_no_export_on_the_stick_fails_with_a_clear_message() {
    let rig = Rig::new();
    let on_stick = rig.import("Night Bus.wav");
    let at_home = rig.import("Other Tune.wav");
    rig.record_stems(&at_home, "Other Tune.wav", FRAMES);
    rig.export(&on_stick, "Night Bus.wav");
    rig.mark_spare();

    let coded = rig.write(&at_home, true);
    assert!(!coded.status.success());
    assert_eq!(coded.error_json()["error"]["code"], "track_not_on_stick");

    let plain = rig.write(&at_home, false);
    assert!(!plain.status.success());
    assert_eq!(plain.stdout, "");
    assert!(
        plain.stderr.starts_with(&format!(
            "wreckordbox: track {at_home} \"Other Tune\" has no export on the Stick at {}",
            rig.stick().display()
        )),
        "{}",
        plain.stderr
    );
    assert!(!rig.stick().join("STEMS").exists());
}

#[test]
fn a_stick_with_no_exported_library_or_that_is_not_there_fails() {
    let rig = Rig::new();
    let track = rig.import("Night Bus.wav");
    rig.record_stems(&track, "Night Bus.wav", FRAMES);
    rig.mark_spare();

    let empty = rig.write(&track, true);
    assert!(!empty.status.success());
    assert_eq!(empty.error_json()["error"]["code"], "track_not_on_stick");

    let nowhere = rig.stick().join("no such folder");
    let missing = rig.sandbox.cli(&[
        "--json",
        "stems",
        "write",
        &track,
        nowhere.to_str().unwrap(),
    ]);
    assert!(!missing.status.success());
    assert_eq!(missing.error_json()["error"]["code"], "stick_not_found");
}

#[test]
fn a_track_with_no_stems_in_the_sidecar_fails_with_a_clear_message() {
    let rig = Rig::new();
    let track = rig.import("Night Bus.wav");
    rig.export(&track, "Night Bus.wav");
    rig.mark_spare();

    let coded = rig.write(&track, true);
    assert!(!coded.status.success());
    assert_eq!(coded.error_json()["error"]["code"], "no_current_stems");

    let plain = rig.write(&track, false);
    assert_eq!(
        plain.stderr,
        format!(
            "wreckordbox: track {track} \"Night Bus\" has no Stems in the Sidecar. Run `wreckordbox stems separate {track}` first.\n"
        )
    );
    assert!(!rig.stick().join("STEMS").exists());
}

#[test]
fn stale_stems_are_not_written() {
    // Stems made from audio the track no longer has are not its Stems.
    let rig = Rig::new();
    let track = rig.import("Night Bus.wav");
    rig.record_stems(&track, "Night Bus.wav", FRAMES);
    rig.export(&track, "Night Bus.wav");
    rig.mark_spare();
    write_audio(&rig.audio("Night Bus.wav"), FRAMES - 1, 330.0);

    let run = rig.write(&track, true);

    assert!(!run.status.success());
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "no_current_stems");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("stale"), "{message}");
    assert!(!rig.stick().join("STEMS").exists());
}

#[test]
fn a_track_that_is_not_in_the_library_fails() {
    let (rig, _) = Rig::ready("Night Bus.wav");

    let run = rig.write("999", true);

    assert!(!run.status.success());
    assert_eq!(run.error_json()["error"]["code"], "track_not_found");
    assert!(!rig.stick().join("STEMS").exists());
}

#[test]
fn a_track_whose_name_the_layout_cannot_represent_is_reported_and_skipped_without_failing() {
    // A file name starting with a dot: the deck skips such folders, so no
    // folder can carry this track's stems.
    let (rig, track) = Rig::ready(".night bus.wav");

    let coded = rig.write(&track, true);

    assert!(coded.status.success(), "{}", coded.stderr);
    let skipped = coded.json();
    assert_eq!(skipped["outcome"], "skipped");
    assert_eq!(skipped["folder"], serde_json::Value::Null);
    assert_eq!(skipped["skipped"]["file_name"], ".night bus.wav");
    assert_eq!(skipped["skipped"]["code"], "starts_with_dot");
    let reason = skipped["skipped"]["reason"].as_str().unwrap();
    assert!(reason.contains("dot"), "{reason}");
    assert!(!rig.stick().join("STEMS").exists());

    let plain = rig.write(&track, false);
    assert!(plain.status.success());
    assert_eq!(plain.stderr, "");
    assert!(
        plain.stdout.starts_with(&format!(
            "Skipped track {track} \".night bus\": no Native stems were written, because its file name starts with a dot"
        )),
        "{}",
        plain.stdout
    );
}

#[test]
fn a_name_fat_cannot_store_as_written_gets_the_layouts_underscore() {
    let (rig, track) = Rig::ready("Intro .wav");

    let run = rig.write(&track, true);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.json()["folder"], "STEMS/Intro _");
    let loaded = deck::load(&[rig.stick()], &rig.track_file(&track)).unwrap();
    assert_eq!((loaded.rank, loaded.pads.len()), (1, 4));
}

#[test]
fn a_stick_exported_from_another_library_does_not_hold_this_librarys_track() {
    let (rig, track) = Rig::ready("Night Bus.wav");
    let ours = {
        let db = rbl_db::Library::open(rig.sandbox.location().clone(), rbl_db::OpenMode::ReadOnly)
            .unwrap();
        rbl_db::export_info::db_id(db.connection()).unwrap()
    };
    assert_ne!(ours, 0);
    let name_library = |id: u64| {
        let one = rig.stick().join("PIONEER/rekordbox/exportLibrary.db");
        let db = rbl_onelibrary::playlists::open_read_write(&one).unwrap();
        db.execute("UPDATE content SET masterDbId = ?1", [id as i64])
            .unwrap();
    };

    name_library(ours + 1);
    let other = rig.write(&track, true);
    assert!(!other.status.success());
    assert_eq!(other.error_json()["error"]["code"], "track_not_on_stick");
    assert!(!rig.stick().join("STEMS").exists());

    name_library(ours);
    assert!(rig.write(&track, true).status.success());
}

#[test]
fn a_stick_named_relative_to_where_the_command_is_run_is_found() {
    let (rig, track) = Rig::ready("Night Bus.wav");
    let parent = rig.stick().parent().unwrap();
    let name = rig.stick().file_name().unwrap().to_str().unwrap();

    let run = rig.sandbox.run(
        rig.sandbox
            .command()
            .current_dir(parent)
            .args(["--json", "stems", "write", &track, name]),
    );

    assert!(run.status.success(), "{}", run.stderr);
    assert!(rig.stick().join("STEMS/Night Bus/vocals.ogg").is_file());
}

#[test]
fn an_export_of_other_audio_than_the_stems_were_made_from_is_refused() {
    // The Stick holds an export of the track as it was. Since then the
    // track got other audio and Stems made from that. Writing them would
    // put Stems of one recording beside the other, with a record on the
    // Stick saying they belong together.
    for frames in [FRAMES, FRAMES - 4_410] {
        let rig = Rig::new();
        let track = rig.import("Night Bus.wav");
        rig.export(&track, "Night Bus.wav");
        rig.mark_spare();
        write_audio(&rig.audio("Night Bus.wav"), frames, 330.0);
        rig.record_stems(&track, "Night Bus.wav", frames);
        let before = deck::snapshot(rig.stick());

        let run = rig.write(&track, true);

        assert!(!run.status.success(), "{frames}");
        assert_eq!(run.stdout, "");
        let error = run.error_json();
        assert_eq!(error["error"]["code"], "export_stale", "{frames}");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(message.contains("Export it again"), "{message}");
        assert_eq!(deck::snapshot(rig.stick()), before);
    }
}

#[test]
fn stems_far_from_their_parents_length_are_refused_and_a_codecs_padding_is_not() {
    // The Sidecar says how long the engine found the parent; the file
    // says how long it is. A decoder may drop a codec's padding (some
    // thousand frames at most), never a second.
    let short = |by: u32| {
        let rig = Rig::new();
        let track = rig.import("Night Bus.wav");
        rig.record_stems(&track, "Night Bus.wav", FRAMES - by);
        rig.export(&track, "Night Bus.wav");
        rig.mark_spare();
        let run = rig.write(&track, true);
        (rig, run)
    };

    let (rig, far) = short(22_050);
    assert!(!far.status.success());
    let error = far.error_json();
    assert_eq!(error["error"]["code"], "stem_length_mismatch");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("22050"), "{message}");
    assert!(!rig.stick().join("STEMS").exists());

    let (_rig, near) = short(1_105);
    assert!(near.status.success(), "{}", near.stderr);
    assert_eq!(near.json()["samples"], FRAMES - 1_105);
}

#[test]
fn another_client_is_told_when_the_command_finds_the_stems_stale() {
    let rig = Rig::new();
    let track = rig.import("Night Bus.wav");
    rig.record_stems(&track, "Night Bus.wav", FRAMES);
    rig.export(&track, "Night Bus.wav");
    rig.mark_spare();
    write_audio(&rig.audio("Night Bus.wav"), FRAMES - 1, 330.0);
    rig.sandbox.status();
    let mut watcher = rig.sandbox.client();
    watcher.subscribe().unwrap();

    let run = rig.write(&track, true);

    assert_eq!(run.error_json()["error"]["code"], "no_current_stems");
    let mut events = Vec::new();
    while let Some(event) = watcher
        .next_event(std::time::Duration::from_millis(500))
        .unwrap()
    {
        events.push(event);
    }
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].event, "sidecar_changed");
    assert_eq!(events[0].data["track_id"], track.as_str());
    assert_eq!(events[0].data["kind"], "stems");
    assert_eq!(events[0].data["fact_id"], serde_json::Value::Null);
    assert_eq!(events[0].data["stale"].as_array().unwrap().len(), 1);

    // Found out once: asking again tells nobody anything new.
    rig.write(&track, true);
    assert!(watcher
        .next_event(std::time::Duration::from_millis(300))
        .unwrap()
        .is_none());
}

#[test]
fn a_stems_folder_that_leads_off_the_stick_is_refused_with_its_own_code() {
    let (rig, track) = Rig::ready("Night Bus.wav");
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("keep.txt"), b"elsewhere").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), rig.stick().join("STEMS")).unwrap();
    let before = deck::snapshot(elsewhere.path());

    let run = rig.write(&track, true);

    assert!(!run.status.success());
    assert_eq!(run.error_json()["error"]["code"], "stick_unsafe");
    assert_eq!(deck::snapshot(elsewhere.path()), before);
}
