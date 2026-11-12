//! The Sidecar travels with backups: what a backup holds of the folder
//! beside the Library, and what a restore puts back.
#![allow(clippy::unwrap_used)]

#[path = "../../wreckordbox-sidecar/tests/common/mod.rs"]
mod audio;

use rbl_backup::restore::{restore, Parts, Phase, Request};
use rbl_backup::sidecar_part::{Contents, Options};
use rbl_backup::{testing, Error};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use wreckordbox_sidecar::stems::{NewStems, Role, StemSet};
use wreckordbox_sidecar::{fingerprint_file, Fact, Fingerprint, Kind, NewFact, Run, RunId, Sidecar, Source, SourceKind};

/// The names of an archive's entries.
fn entries(archive: &Path) -> Vec<String> {
    let zip = rbl_backup::archive::open(archive).unwrap();
    let mut names = rbl_backup::archive::names(&zip);
    names.sort();
    names
}

/// A Library with a Sidecar beside it: a Fact from the DJ in its own Run,
/// one track's Stems, a table of a later version, a kinds file, a Recipe in
/// a folder, and data that can be made again.
struct Fixture {
    dir: tempfile::TempDir,
    location: rbl_db::LibraryLocation,
    sidecar_dir: PathBuf,
    archive: PathBuf,
    /// The fingerprint of the audio of the track that has Stems.
    parent: Fingerprint,
}

const KINDS: &str = "[kinds.era]\nnature = \"judgement\"\nvalue = \"text\"\n";
const TAGGED: &str = "10001";
const SEPARATED: &str = "10003";
const DERIVED: [&str; 4] = ["auditions/10001/vocals.wav", "renders/abc.wav", "cache/x", "work/run-9/part.wav"];

fn dj() -> Source {
    Source { kind: SourceKind::Dj, name: "dj".to_owned() }
}

fn record(sidecar: &Sidecar, track: &str, kind: &str, value: serde_json::Value) -> RunId {
    let run = sidecar.begin_run(&dj(), "Tag by hand").unwrap();
    sidecar
        .record(&NewFact { track_id: track, fingerprint: None, kind: Kind::judgement(kind), value, source: &dj(), confidence: 1.0, run })
        .unwrap();
    sidecar.finish_run(run, "recorded").unwrap();
    run
}

fn separate(sidecar: &Sidecar, parent: &Fingerprint) -> StemSet {
    let source = Source { kind: SourceKind::Tool, name: "separation-engine".to_owned() };
    let run = sidecar.begin_run(&source, "Separate").unwrap();
    let workspace = sidecar.workspace(run).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .map(|role| {
            let path = workspace.join(format!("{role}.wav"));
            audio::write_audio(&path, 500, 0, None);
            (role, path)
        })
        .collect();
    let stems = sidecar
        .record_stems(&NewStems {
            track_id: SEPARATED,
            fingerprint: parent,
            parent_samples: 500,
            sample_rate: 44_100,
            files: &files,
            source: &source,
            produced_by: serde_json::json!({ "model": "a-model" }),
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "recorded").unwrap();
    stems
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let location = testing::library(dir.path());
    testing::set_rating(&location, 4);
    let sidecar_dir = wreckordbox_sidecar::default_dir(&location.master_db);
    let parent_audio = dir.path().join("parent.wav");
    audio::write_audio(&parent_audio, 500, 3, None);
    let parent = fingerprint_file(&parent_audio).unwrap();
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    record(&sidecar, TAGGED, "genre", serde_json::json!("house"));
    separate(&sidecar, &parent);
    drop(sidecar);
    // What a later version keeps in the same database travels by rule.
    let db = rusqlite::Connection::open(sidecar_dir.join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TABLE mutant (id INTEGER PRIMARY KEY, playlist TEXT, edits TEXT); INSERT INTO mutant VALUES (1, 'Friday', 'no vocals');").unwrap();
    drop(db);
    fs::write(sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE), KINDS).unwrap();
    fs::create_dir_all(sidecar_dir.join("recipes/empty")).unwrap();
    fs::write(sidecar_dir.join("recipes/friday.toml"), "warm up").unwrap();
    for derived in DERIVED {
        let path = sidecar_dir.join(derived);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"derived audio").unwrap();
    }
    let archive = dir.path().join("backup.zip");
    Fixture { dir, location, sidecar_dir, archive, parent }
}

/// What the Sidecar knows, read through its own interface.
#[derive(Debug, PartialEq)]
struct Known {
    facts: Vec<Fact>,
    runs: Vec<Option<Run>>,
    mutants: Vec<(i64, String, String)>,
}

impl Fixture {
    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn known(&self) -> Known {
        let sidecar = Sidecar::open(&self.sidecar_dir).unwrap();
        let mut facts = sidecar.facts_of_track(TAGGED).unwrap();
        facts.extend(sidecar.facts_of_track(SEPARATED).unwrap());
        let runs = (1..=4).map(|id| sidecar.run(RunId(id)).unwrap()).collect();
        drop(sidecar);
        let db = rusqlite::Connection::open(self.sidecar_dir.join("sidecar.db")).unwrap();
        let mutants = db
            .prepare("SELECT id, playlist, edits FROM mutant")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        Known { facts, runs, mutants }
    }

    fn stems(&self) -> Option<StemSet> {
        Sidecar::open(&self.sidecar_dir).unwrap().current_stems(SEPARATED, &self.parent).unwrap()
    }

    /// Everything a backup holds is changed after it was taken.
    fn change(&self) {
        testing::set_rating(&self.location, 1);
        let sidecar = Sidecar::open(&self.sidecar_dir).unwrap();
        record(&sidecar, TAGGED, "genre", serde_json::json!("techno"));
        drop(sidecar);
        let db = rusqlite::Connection::open(self.sidecar_dir.join("sidecar.db")).unwrap();
        db.execute("UPDATE mutant SET edits = 'no drums'", []).unwrap();
        drop(db);
        fs::write(self.sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE), "edited").unwrap();
        fs::write(self.sidecar_dir.join("tag-schema.toml"), "written later").unwrap();
        fs::remove_file(self.sidecar_dir.join("recipes/friday.toml")).unwrap();
        fs::write(self.sidecar_dir.join("recipes/saturday.toml"), "written later").unwrap();
    }

    fn restore(&self, archive: &Path, parts: Parts) -> rbl_backup::Result<Vec<Phase>> {
        let mut phases = Vec::new();
        restore(
            &Request { archive, parts, location: &self.location, state_dir: &self.state(), sizes: None },
            &|| Ok(()),
            &mut |phase, _, _, _| {
                if phases.last() != Some(&phase) {
                    phases.push(phase);
                }
                Ok(())
            },
        )?;
        Ok(phases)
    }

    /// Names a restore left in the Sidecar's folder, which it must not.
    fn leftovers(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.sidecar_dir) else { return Vec::new() };
        entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(rbl_backup::journal::PREFIX))
            .collect()
    }

    /// A copy of the fixture's backup with more entries written into it.
    fn tampered(&self, extra: &[(&str, &[u8])]) -> PathBuf {
        let bad = self.dir.path().join("tampered.zip");
        fs::copy(&self.archive, &bad).unwrap();
        let mut zip = zip::ZipWriter::new_append(fs::OpenOptions::new().read(true).write(true).open(&bad).unwrap()).unwrap();
        for (name, bytes) in extra {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        bad
    }
}

#[test]
fn a_backup_holds_the_sidecars_database_and_the_djs_files_but_nothing_that_can_be_made_again() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    let sidecar: Vec<String> = entries(&fixture.archive).into_iter().filter(|name| name.starts_with("sidecar")).collect();
    assert_eq!(
        sidecar,
        ["sidecar.json", "sidecar/", "sidecar/kinds.toml", "sidecar/recipes/", "sidecar/recipes/empty/", "sidecar/recipes/friday.toml", "sidecar/sidecar.db"]
    );
    let contents = Contents::read(&fixture.archive).unwrap().unwrap();
    assert!(!contents.stems);
    // Four Stems of 500 stereo 16-bit frames and a 44-byte header each.
    assert_eq!(contents.stems_bytes, 4 * (500 * 4 + 44));
}

#[test]
fn a_restore_puts_the_sidecar_back_as_it_was_with_the_library_it_was_taken_with() {
    let fixture = fixture();
    let before = fixture.known();
    assert_eq!(before.facts.len(), 2);
    assert_eq!(before.mutants, [(1, "Friday".to_owned(), "no vocals".to_owned())]);
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    assert_ne!(fixture.known(), before);

    let phases = fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert_eq!(phases, [Phase::Preparing, Phase::Unpacking, Phase::Validating, Phase::Replacing]);
    assert_eq!(testing::rating(&fixture.location), 4);
    assert_eq!(fixture.known(), before);
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE)).unwrap(), KINDS);
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join("recipes/friday.toml")).unwrap(), "warm up");
    assert!(fixture.sidecar_dir.join("recipes/empty").is_dir());
    // What was written after the backup is not part of the Library it was taken with.
    assert!(!fixture.sidecar_dir.join("tag-schema.toml").exists());
    assert!(!fixture.sidecar_dir.join("recipes/saturday.toml").exists());
    // What a backup never holds is left where it is.
    for derived in DERIVED.iter().filter(|derived| !derived.starts_with("work/")) {
        assert_eq!(fs::read(fixture.sidecar_dir.join(derived)).unwrap(), b"derived audio", "{derived}");
    }
    assert_eq!(fixture.leftovers(), [] as [String; 0]);
    assert!(!rbl_backup::journal::pending(&fixture.state()));

    // On a machine with no Sidecar yet, the folder is made.
    fs::remove_dir_all(&fixture.sidecar_dir).unwrap();
    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert_eq!(fixture.known(), before);
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE)).unwrap(), KINDS);
}

#[test]
fn the_sidecar_goes_back_with_the_database_and_not_with_any_other_part() {
    let fixture = fixture();
    let before = fixture.known();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let changed = fixture.known();
    fixture.restore(&fixture.archive, Parts { analysis: true, artwork: true, library_files: true, ..Parts::default() }).unwrap();
    assert_eq!(fixture.known(), changed);
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join("tag-schema.toml")).unwrap(), "written later");
    fixture.restore(&fixture.archive, Parts { database: true, ..Parts::default() }).unwrap();
    assert_eq!(fixture.known(), before);
}

#[test]
fn a_backup_taken_before_backups_carried_the_sidecar_still_restores_and_leaves_the_sidecar_alone() {
    let fixture = fixture();
    // As every earlier backup is: the Library's entries and no Sidecar.
    let aside = fixture.dir.path().join("aside");
    fs::rename(&fixture.sidecar_dir, &aside).unwrap();
    testing::archive(&fixture.location, &fixture.archive, None);
    fs::rename(&aside, &fixture.sidecar_dir).unwrap();
    assert!(!entries(&fixture.archive).iter().any(|name| name.starts_with("sidecar")));
    assert_eq!(Contents::read(&fixture.archive).unwrap(), None);
    fixture.change();
    let changed = fixture.known();

    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert_eq!(testing::rating(&fixture.location), 4);
    assert_eq!(fixture.known(), changed);
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join("tag-schema.toml")).unwrap(), "written later");
    assert!(fixture.stems().is_some());
}

#[test]
fn stems_whose_audio_was_not_in_the_backup_read_as_needing_separation_after_a_restore() {
    let fixture = fixture();
    assert!(fixture.stems().is_some());
    testing::archive(&fixture.location, &fixture.archive, None);
    assert!(!entries(&fixture.archive).iter().any(|name| name.starts_with("sidecar/stems")));

    // Stem audio still on disk is left there and is still the track's Stems.
    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert!(fixture.stems().is_some_and(|set| set.stems.iter().all(|stem| stem.path.is_file())));

    // On a machine without it, the restored database still records the
    // separation, but the track has no Stems until it is separated again.
    fs::remove_dir_all(fixture.sidecar_dir.join("stems")).unwrap();
    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert_eq!(fixture.stems(), None);
    let sidecar = Sidecar::open(&fixture.sidecar_dir).unwrap();
    let recorded = sidecar.stems(SEPARATED).unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(!recorded[0].audio_present());
}

#[test]
fn stem_audio_travels_when_the_backup_is_asked_to_include_it() {
    let fixture = fixture();
    let before = fixture.stems().unwrap();
    testing::archive_with(&fixture.location, &fixture.archive, None, Options { stems: true });
    let contents = Contents::read(&fixture.archive).unwrap().unwrap();
    assert!(contents.stems);
    assert!(contents.bytes > contents.stems_bytes && contents.stems_bytes == 4 * (500 * 4 + 44));
    assert!(entries(&fixture.archive).contains(&"sidecar/stems/10003/run-2/vocals.wav".to_owned()));

    let vocals = before.stems[0].path.clone();
    let original = fs::read(&vocals).unwrap();
    fs::remove_dir_all(fixture.sidecar_dir.join("stems")).unwrap();
    // Stem audio made after the backup is not the restored Library's.
    fs::create_dir_all(fixture.sidecar_dir.join("stems/10009/run-8")).unwrap();
    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert_eq!(fixture.stems(), Some(before));
    assert_eq!(fs::read(&vocals).unwrap(), original);
    assert!(!fixture.sidecar_dir.join("stems/10009").exists());
}

/// The fixture's backup with one entry's contents replaced.
fn rewritten(fixture: &Fixture, name: &str, contents: &[u8]) -> PathBuf {
    use std::io::Read;
    let bad = fixture.dir.path().join("rewritten.zip");
    let mut from = rbl_backup::archive::open(&fixture.archive).unwrap();
    let mut to = zip::ZipWriter::new(fs::File::create(&bad).unwrap());
    for index in 0..from.len() {
        let mut entry = from.by_index(index).unwrap();
        let entry_name = entry.name().to_owned();
        if entry_name.ends_with('/') {
            to.add_directory(entry_name, zip::write::SimpleFileOptions::default()).unwrap();
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        to.start_file(&entry_name, zip::write::SimpleFileOptions::default()).unwrap();
        to.write_all(if entry_name == name { contents } else { &bytes }).unwrap();
    }
    to.finish().unwrap();
    bad
}

/// A restore that must be refused, leaving the Library, the Sidecar and
/// the folders around them as they were.
fn refused(fixture: &Fixture, archive: &Path, changed: &Known) -> String {
    let error = fixture.restore(archive, Parts::ALL).unwrap_err();
    assert_eq!(testing::rating(&fixture.location), 1);
    assert_eq!(&fixture.known(), changed);
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE)).unwrap(), "edited");
    assert_eq!(fixture.leftovers(), [] as [String; 0]);
    assert!(!rbl_backup::journal::pending(&fixture.state()));
    error.to_string()
}

#[test]
fn an_archive_cannot_write_outside_the_sidecar_through_its_sidecar_entries() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let changed = fixture.known();

    for name in ["sidecar/../../escape", "sidecar/recipes/../../../escape", "sidecar/../escape", "sidecar//etc/escape/../../../../escape"] {
        let bad = fixture.tampered(&[(name, b"bad")]);
        assert_eq!(refused(&fixture, &bad, &changed), "The backup has an unsafe entry.", "{name}");
    }
    for outside in [fixture.dir.path().join("escape"), fixture.sidecar_dir.parent().unwrap().join("escape"), fixture.sidecar_dir.parent().unwrap().parent().unwrap().join("escape")] {
        assert!(!outside.exists(), "{}", outside.display());
    }

    // A stored symbolic link is never unpacked, here as elsewhere.
    let bad = fixture.dir.path().join("link.zip");
    fs::copy(&fixture.archive, &bad).unwrap();
    let mut zip = zip::ZipWriter::new_append(fs::OpenOptions::new().read(true).write(true).open(&bad).unwrap()).unwrap();
    zip.add_symlink("sidecar/recipes/link", "/etc", zip::write::SimpleFileOptions::default()).unwrap();
    zip.finish().unwrap();
    assert_eq!(refused(&fixture, &bad, &changed), "The backup has an unsafe entry.");

    // One name as both a file and a folder.
    let bad = fixture.tampered(&[("sidecar/kinds.toml/inner", b"bad")]);
    assert_eq!(refused(&fixture, &bad, &changed), "The backup has an unsafe entry.");
}

#[test]
fn entries_a_backup_never_holds_are_not_unpacked_even_when_an_archive_has_them() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    let vocals = fixture.stems().unwrap().stems[0].path.clone();
    let original = fs::read(&vocals).unwrap();
    let bad = fixture.tampered(&[
        ("sidecar/work/planted", b"bad"),
        ("sidecar/auditions/planted", b"bad"),
        ("sidecar/renders/planted", b"bad"),
        ("sidecar/cache/planted", b"bad"),
        ("sidecar/.hidden", b"bad"),
        ("sidecar/.rbxport-restore-planted-new", b"bad"),
        // The record says Stem audio is not in this backup.
        ("sidecar/stems/10003/run-2/vocals.wav", b"bad"),
        ("sidecar/stems/planted", b"bad"),
        ("sidecars/planted", b"bad"),
    ]);
    fixture.restore(&bad, Parts::ALL).unwrap();
    for planted in ["work/planted", "auditions/planted", "renders/planted", "cache/planted", ".hidden", ".rbxport-restore-planted-new", "stems/planted"] {
        assert!(!fixture.sidecar_dir.join(planted).exists(), "{planted}");
    }
    assert!(!fixture.sidecar_dir.parent().unwrap().join("sidecars").exists());
    assert_eq!(fs::read(&vocals).unwrap(), original);
    assert!(fixture.stems().is_some());
}

#[test]
fn a_damaged_or_newer_sidecar_in_a_backup_is_refused_before_anything_is_replaced() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let changed = fixture.known();

    let mut garbage = b"SQLite format 3\0".to_vec();
    garbage.extend([7; 4096]);
    let bad = rewritten(&fixture, "sidecar/sidecar.db", &garbage);
    assert_eq!(refused(&fixture, &bad, &changed), "The backup's Sidecar database sidecar.db is damaged.");

    let bad = rewritten(&fixture, "sidecar.json", br#"{"version":2,"bytes":1,"stems":false}"#);
    assert!(refused(&fixture, &bad, &changed).contains("newer version of wreckordbox"));

    let bad = rewritten(&fixture, "sidecar.json", b"not a record");
    assert_eq!(refused(&fixture, &bad, &changed), "The backup's Sidecar record cannot be read.");
}

#[test]
fn stopping_a_restore_leaves_the_sidecar_as_it_was_and_no_staged_files() {
    let fixture = fixture();
    testing::archive_with(&fixture.location, &fixture.archive, None, Options { stems: true });
    fixture.change();
    let changed = fixture.known();
    let result = restore(
        &Request { archive: &fixture.archive, parts: Parts::ALL, location: &fixture.location, state_dir: &fixture.state(), sizes: None },
        &|| Ok(()),
        &mut |phase, done, _, _| if phase == Phase::Unpacking && done > 0 { Err(Error::Cancelled) } else { Ok(()) },
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    // And a guard that refuses at the last moment, with everything unpacked.
    let calls = std::cell::Cell::new(0);
    let result = restore(
        &Request { archive: &fixture.archive, parts: Parts::ALL, location: &fixture.location, state_dir: &fixture.state(), sizes: None },
        &|| {
            calls.set(calls.get() + 1);
            if calls.get() > 1 { Err(Error::Refused("Quit wreckordbox before restoring a backup.".to_owned())) } else { Ok(()) }
        },
        &mut |_, _, _, _| Ok(()),
    );
    assert!(matches!(result, Err(Error::Refused(_))));
    assert_eq!(testing::rating(&fixture.location), 1);
    assert_eq!(fixture.known(), changed);
    assert!(fixture.stems().is_some());
    assert_eq!(fixture.leftovers(), [] as [String; 0]);
}

fn journal(fixture: &Fixture, committed: bool, target: &Path) -> (PathBuf, PathBuf) {
    use rbl_backup::journal::{Journal, Swap};
    let parent = target.parent().unwrap();
    let (staged, previous) = (parent.join(".rbxport-restore-test-new"), parent.join(".rbxport-restore-test-old"));
    let journal = Journal {
        library: fixture.location.master_db.clone(),
        committed,
        swaps: vec![Swap { target: target.to_path_buf(), staged: staged.clone(), previous: previous.clone(), existed: true }],
    };
    fs::create_dir_all(fixture.state()).unwrap();
    fs::write(rbl_backup::journal::path(&fixture.state()), serde_json::to_vec(&journal).unwrap()).unwrap();
    (staged, previous)
}

#[test]
fn an_interrupted_restore_of_the_sidecar_rolls_back_and_a_committed_one_is_finished() {
    let fixture = fixture();
    let kinds = fixture.sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE);
    for committed in [false, true] {
        fs::write(&kinds, "before").unwrap();
        let (staged, previous) = journal(&fixture, committed, &kinds);
        fs::write(&staged, "after").unwrap();
        fs::rename(&kinds, &previous).unwrap();
        fs::rename(&staged, &kinds).unwrap();
        rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
        assert_eq!(fs::read_to_string(&kinds).unwrap(), if committed { "after" } else { "before" });
        assert_eq!(fixture.leftovers(), [] as [String; 0]);
        assert!(!rbl_backup::journal::pending(&fixture.state()));
    }
}

#[test]
fn a_journal_cannot_name_what_a_backup_never_holds_or_a_path_outside_the_sidecar() {
    let fixture = fixture();
    let beside = fixture.sidecar_dir.parent().unwrap().join("notes.txt");
    fs::write(&beside, "keep").unwrap();
    let inside = fixture.sidecar_dir.join("recipes/friday.toml");
    for target in [fixture.sidecar_dir.join("work"), fixture.sidecar_dir.join("renders"), fixture.sidecar_dir.join(".git"), inside.clone(), beside.clone(), fixture.sidecar_dir.clone()] {
        let (_, previous) = journal(&fixture, false, &target);
        fs::write(&previous, "planted").unwrap();
        assert!(rbl_backup::journal::recover(&fixture.state(), &fixture.location).is_err(), "{}", target.display());
        fs::remove_file(&previous).unwrap();
    }
    assert_eq!(fs::read_to_string(&beside).unwrap(), "keep");
    assert_eq!(fs::read_to_string(&inside).unwrap(), "warm up");
    assert!(fixture.sidecar_dir.join("work").is_dir() && fixture.sidecar_dir.join("renders").is_dir());
}

#[test]
fn the_sidecars_folder_is_the_one_the_sidecar_itself_uses() {
    let master = Path::new("/library/master.db");
    assert_eq!(rbl_backup::sidecar_part::FOLDER_NAME, wreckordbox_sidecar::FOLDER_NAME);
    assert_eq!(rbl_backup::sidecar_part::dir_from(master, None), wreckordbox_sidecar::default_dir(master));
}
