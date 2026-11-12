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
use wreckordbox_sidecar::mutants::{InRun, Mutant, MutantChange, MutantUpdate, NewMutant};
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
/// one track's Stems, a Mutant of a playlist entry, a table a later version
/// might add (proof that a table this code does not know travels), a kinds
/// file, a Recipe in a folder, and data
/// that can be made again.
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
    separate_as(sidecar, parent, 0)
}

/// A separation whose Stems' samples all have `value`.
fn separate_as(sidecar: &Sidecar, parent: &Fingerprint, value: i16) -> StemSet {
    let source = Source { kind: SourceKind::Tool, name: "separation-engine".to_owned() };
    let run = sidecar.begin_run(&source, "Separate").unwrap();
    let workspace = sidecar.workspace(run).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .map(|role| {
            let path = workspace.join(format!("{role}.wav"));
            audio::write_audio(&path, 500, value, None);
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
    fixture_with(true)
}

/// The fixture, with or without its separation.
fn fixture_with(separated: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let location = testing::library(dir.path());
    testing::set_rating(&location, 4);
    let sidecar_dir = wreckordbox_sidecar::default_dir(&location.master_db);
    let parent_audio = dir.path().join("parent.wav");
    audio::write_audio(&parent_audio, 500, 3, None);
    let parent = fingerprint_file(&parent_audio).unwrap();
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    record(&sidecar, TAGGED, "genre", serde_json::json!("house"));
    if separated {
        separate(&sidecar, &parent);
    }
    drop(sidecar);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    sidecar
        .create_mutant(&NewMutant {
            playlist_id: "900001",
            entry_id: "entry-a",
            track_id: SEPARATED,
            fingerprint: Some(&parent),
            edits: serde_json::json!({ "stems": { "vocals": false } }),
            source: &dj(),
            run: InRun::Own { description: "Mutate" },
        })
        .unwrap();
    drop(sidecar);
    // What a later version keeps in the same database travels by rule.
    let db = rusqlite::Connection::open(sidecar_dir.join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TABLE later_table (id INTEGER PRIMARY KEY, playlist TEXT, edits TEXT); INSERT INTO later_table VALUES (1, 'Friday', 'no vocals');").unwrap();
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

/// What the Sidecar knows, read through its own interface, and the rows of
/// the stand-in table.
#[derive(Debug, PartialEq)]
struct Known {
    facts: Vec<Fact>,
    runs: Vec<Option<Run>>,
    mutants: Vec<(Mutant, Vec<MutantChange>)>,
    stand_in_rows: Vec<(i64, String, String)>,
}

impl Fixture {
    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn known(&self) -> Known {
        let sidecar = Sidecar::open(&self.sidecar_dir).unwrap();
        let mut facts = sidecar.facts_of_track(TAGGED).unwrap();
        facts.extend(sidecar.facts_of_track(SEPARATED).unwrap());
        let runs = (1..=6).map(|id| sidecar.run(RunId(id)).unwrap()).collect();
        let mutants = sidecar
            .mutants(None)
            .unwrap()
            .into_iter()
            .map(|mutant| {
                let changes = sidecar.mutant_changes(mutant.id).unwrap();
                (mutant, changes)
            })
            .collect();
        drop(sidecar);
        let db = rusqlite::Connection::open(self.sidecar_dir.join("sidecar.db")).unwrap();
        let stand_in_rows = db
            .prepare("SELECT id, playlist, edits FROM later_table")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        Known { facts, runs, mutants, stand_in_rows }
    }

    fn stems(&self) -> Option<StemSet> {
        Sidecar::open(&self.sidecar_dir).unwrap().current_stems(SEPARATED, &self.parent).unwrap()
    }

    /// Everything a backup holds is changed after it was taken.
    fn change(&self) {
        testing::set_rating(&self.location, 1);
        let sidecar = Sidecar::open(&self.sidecar_dir).unwrap();
        record(&sidecar, TAGGED, "genre", serde_json::json!("techno"));
        if let Some(mutant) = sidecar.mutant_of_entry("entry-a").unwrap() {
            let update = MutantUpdate { fingerprint: None, source: &dj(), run: InRun::Own { description: "Edit" } };
            sidecar.edit_mutant(mutant.id, &serde_json::json!({ "key_shift": 2 }), &update).unwrap();
        }
        drop(sidecar);
        let db = rusqlite::Connection::open(self.sidecar_dir.join("sidecar.db")).unwrap();
        db.execute("UPDATE later_table SET edits = 'no drums'", []).unwrap();
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
fn a_restore_puts_facts_runs_and_an_unknown_table_back_as_they_were_with_the_library_they_were_taken_with() {
    let fixture = fixture();
    let before = fixture.known();
    assert_eq!(before.facts.len(), 2);
    assert_eq!(before.mutants.len(), 1);
    assert_eq!(before.mutants[0].0.edits, serde_json::json!({ "stems": { "vocals": false } }));
    assert_eq!(before.stand_in_rows, [(1, "Friday".to_owned(), "no vocals".to_owned())]);
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

/// Restoring a Sidecar rewinds Run identifiers, and Stem folders are named
/// after Runs: a separation made after restoring an older backup can write
/// other audio at the very paths a newer backup's Stems record. Restoring
/// that newer backup without its Stem audio must not take those files for
/// its Stems.
#[test]
fn stem_audio_left_on_disk_by_a_later_separation_at_the_same_paths_is_not_taken_for_a_restored_backups_stems() {
    let fixture = fixture_with(false);
    let older = fixture.dir.path().join("older.zip");
    testing::archive(&fixture.location, &older, None);
    let sidecar = Sidecar::open(&fixture.sidecar_dir).unwrap();
    let saved = separate_as(&sidecar, &fixture.parent, 0);
    drop(sidecar);
    testing::archive(&fixture.location, &fixture.archive, None);

    fixture.restore(&older, Parts::ALL).unwrap();
    assert_eq!(fixture.stems(), None);
    let sidecar = Sidecar::open(&fixture.sidecar_dir).unwrap();
    let later = separate_as(&sidecar, &fixture.parent, 9);
    drop(sidecar);
    // The Run was numbered again: the later Stems are at the same paths.
    let paths = |set: &StemSet| set.stems.iter().map(|stem| stem.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&later), paths(&saved));

    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert!(paths(&saved).iter().all(|path| path.is_file()));
    assert_eq!(fixture.stems(), None);
    let recorded = Sidecar::open(&fixture.sidecar_dir).unwrap().stems(SEPARATED).unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(!recorded[0].audio_present());
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

    // The record's size is the archive's word: an absurd one is refused, not trusted.
    let bad = rewritten(&fixture, "sidecar.json", br#"{"version":1,"bytes":18446744073709551615,"stems":false}"#);
    assert!(refused(&fixture, &bad, &changed).contains("not enough free disk space to restore the Sidecar"));

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
        sidecar: Some(fixture.sidecar_dir.canonicalize().unwrap()),
        owed: None,
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

/// A restore into a Sidecar folder `WRECKORDBOX_SIDECAR_DIR` named, which
/// the program recovering after it does not name: the journal's own record
/// of the folder is what recovery acts on.
#[test]
fn an_interrupted_restore_into_a_sidecar_folder_named_elsewhere_rolls_back_without_that_setting() {
    let fixture = fixture();
    let chosen = fixture.dir.path().join("chosen-sidecar");
    fs::create_dir_all(&chosen).unwrap();
    let kinds = chosen.join(wreckordbox_sidecar::KINDS_FILE);
    fs::write(&kinds, "before").unwrap();
    let (staged, previous) = journal(&fixture, false, &kinds);
    let mut recorded: rbl_backup::journal::Journal = serde_json::from_slice(&fs::read(rbl_backup::journal::path(&fixture.state())).unwrap()).unwrap();
    recorded.sidecar = Some(chosen.canonicalize().unwrap());
    recorded.swaps[0].target = chosen.canonicalize().unwrap().join(wreckordbox_sidecar::KINDS_FILE);
    recorded.swaps[0].staged = chosen.canonicalize().unwrap().join(staged.file_name().unwrap());
    recorded.swaps[0].previous = chosen.canonicalize().unwrap().join(previous.file_name().unwrap());
    fs::write(rbl_backup::journal::path(&fixture.state()), serde_json::to_vec(&recorded).unwrap()).unwrap();
    fs::write(chosen.join(staged.file_name().unwrap()), "after").unwrap();
    fs::rename(&kinds, chosen.join(previous.file_name().unwrap())).unwrap();
    fs::rename(chosen.join(staged.file_name().unwrap()), &kinds).unwrap();
    assert!(std::env::var_os(rbl_backup::sidecar_part::DIR_ENV).is_none());

    rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
    rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
    assert_eq!(fs::read_to_string(&kinds).unwrap(), "before");
    assert_eq!(fs::read_dir(&chosen).unwrap().count(), 1);
    assert!(!rbl_backup::journal::pending(&fixture.state()));
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

/// The fixture's Sidecar folder moved aside and a symbolic link put in its
/// place, pointing at a folder that also holds a file of the DJ's own.
fn redirect(fixture: &Fixture) -> PathBuf {
    let elsewhere = fixture.dir.path().join("elsewhere");
    fs::rename(&fixture.sidecar_dir, &elsewhere).unwrap();
    fs::write(elsewhere.join("notes.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, &fixture.sidecar_dir).unwrap();
    elsewhere
}

#[test]
fn a_sidecar_folder_that_is_a_symbolic_link_is_refused_by_backup_and_restore_and_nothing_is_written_through_it() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let elsewhere = redirect(&fixture);
    let listing = |dir: &Path| {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    };
    let before = listing(&elsewhere);

    let error = fixture.restore(&fixture.archive, Parts::ALL).unwrap_err().to_string();
    assert!(error.contains("symbolic link"), "{error}");
    assert_eq!(testing::rating(&fixture.location), 1);
    assert_eq!(listing(&elsewhere), before);
    assert_eq!(fs::read_to_string(elsewhere.join("notes.txt")).unwrap(), "keep");
    assert!(!rbl_backup::journal::pending(&fixture.state()));

    let error = rbl_backup::sidecar_part::collect_for(&fixture.location, Options::default(), &mut || true).unwrap_err().to_string();
    assert!(error.contains("symbolic link"), "{error}");
    assert!(rbl_backup::sidecar_part::collect(&fixture.sidecar_dir, Options::default(), &mut || true).is_err());

    // A journal naming the folder before it was redirected is not acted on.
    let (_, previous) = journal(&fixture, false, &elsewhere.join("kinds.toml"));
    drop(previous);
    let mut recorded: rbl_backup::journal::Journal = serde_json::from_slice(&fs::read(rbl_backup::journal::path(&fixture.state())).unwrap()).unwrap();
    recorded.sidecar = Some(fixture.sidecar_dir.clone());
    recorded.swaps[0].target = fixture.sidecar_dir.join("kinds.toml");
    fs::write(rbl_backup::journal::path(&fixture.state()), serde_json::to_vec(&recorded).unwrap()).unwrap();
    assert!(rbl_backup::journal::recover(&fixture.state(), &fixture.location).is_err());
    assert_eq!(listing(&elsewhere), before);
}

#[test]
fn a_sidecar_folder_overlapping_the_library_is_refused() {
    use rbl_backup::sidecar_part::Destination;
    let fixture = fixture();
    let root = fixture.location.master_db.parent().unwrap().to_path_buf();
    for overlapping in [
        root.clone(),
        root.parent().unwrap().to_path_buf(),
        fixture.location.share_root.clone(),
        fixture.location.share_root.join("PIONEER/USBANLZ/sidecar"),
        fixture.location.share_root.join("PIONEER"),
        root.join("master.db"),
        root.join("masterPlaylists6.xml"),
    ] {
        assert!(Destination::check(&fixture.location, &overlapping).is_err(), "{}", overlapping.display());
    }
    assert!(Destination::check(&fixture.location, Path::new("relative/sidecar")).is_err());
    assert!(Destination::check(&fixture.location, &root.join("a/../sidecar")).is_err());
    let checked = Destination::check(&fixture.location, &fixture.sidecar_dir).unwrap();
    assert_eq!(checked.path, fixture.sidecar_dir.canonicalize().unwrap());
    // A folder of its own elsewhere, made by the restore if need be.
    assert!(Destination::check(&fixture.location, &fixture.dir.path().join("sidecar-elsewhere/new")).is_ok());
}

/// The fixture's `sidecar.db` bytes with the SQLite header zeroed.
fn headerless(fixture: &Fixture) -> Vec<u8> {
    let mut bytes = fs::read(fixture.sidecar_dir.join("sidecar.db")).unwrap();
    bytes[..16].fill(0);
    bytes
}

#[test]
fn an_empty_or_headerless_sidecar_database_in_a_backup_is_refused_before_anything_is_replaced() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    let headerless = headerless(&fixture);
    fixture.change();
    let changed = fixture.known();
    for contents in [b"".as_slice(), headerless.as_slice(), b"not a database"] {
        let bad = rewritten(&fixture, "sidecar/sidecar.db", contents);
        assert_eq!(refused(&fixture, &bad, &changed), "The backup's Sidecar database sidecar.db is damaged.");
    }
    // A sound SQLite database that is not a Sidecar store.
    let other = fixture.dir.path().join("other.db");
    rusqlite::Connection::open(&other).unwrap().execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);").unwrap();
    let bad = rewritten(&fixture, "sidecar/sidecar.db", &fs::read(&other).unwrap());
    assert_eq!(refused(&fixture, &bad, &changed), "The backup's Sidecar database sidecar.db is damaged.");
}

#[test]
fn a_nested_database_that_is_damaged_or_missing_in_a_backup_is_refused() {
    let fixture = fixture();
    let nested = fixture.sidecar_dir.join("recipes/index.db");
    rusqlite::Connection::open(&nested).unwrap().execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE recipe (name TEXT); INSERT INTO recipe VALUES ('friday');").unwrap();
    testing::archive(&fixture.location, &fixture.archive, None);
    let contents = Contents::read(&fixture.archive).unwrap().unwrap();
    assert_eq!(contents.databases, ["recipes/index.db", "sidecar.db"]);
    assert!(!entries(&fixture.archive).iter().any(|name| name.ends_with("-wal") || name.ends_with("-shm")));
    let mut bytes = fs::read(&nested).unwrap();
    bytes[..16].fill(0);
    fixture.change();
    let changed = fixture.known();
    for damaged in [b"".as_slice(), bytes.as_slice()] {
        let bad = rewritten(&fixture, "sidecar/recipes/index.db", damaged);
        assert_eq!(refused(&fixture, &bad, &changed), "The backup's Sidecar database recipes/index.db is damaged.");
    }
    // A record naming a database the archive does not hold.
    let mut record = contents.clone();
    record.databases.push("recipes/gone.db".to_owned());
    let bad = rewritten(&fixture, "sidecar.json", &serde_json::to_vec(&record).unwrap());
    assert_eq!(refused(&fixture, &bad, &changed), "The backup's Sidecar database recipes/gone.db is missing.");
}

#[test]
fn a_damaged_sidecar_database_on_disk_fails_the_backup_rather_than_travelling_as_a_file() {
    let fixture = fixture();
    let database = fixture.sidecar_dir.join("sidecar.db");
    let headerless = headerless(&fixture);
    for contents in [b"".as_slice(), headerless.as_slice()] {
        fs::write(&database, contents).unwrap();
        let error = rbl_backup::sidecar_part::collect_for(&fixture.location, Options::default(), &mut || true).unwrap_err().to_string();
        assert!(error.contains("sidecar.db"), "{error}");
    }
}

#[test]
fn a_backup_taken_while_a_writer_holds_the_sidecar_open_has_every_committed_row_and_no_uncommitted_one() {
    let fixture = fixture();
    let before = fixture.known();
    let writer = rusqlite::Connection::open(fixture.sidecar_dir.join("sidecar.db")).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO later_table VALUES (2, 'Saturday', 'committed');").unwrap();
    assert!(fs::metadata(fixture.sidecar_dir.join("sidecar.db-wal")).unwrap().len() > 0);
    writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO later_table VALUES (3, 'Sunday', 'not committed');").unwrap();
    testing::archive(&fixture.location, &fixture.archive, None);
    writer.execute_batch("ROLLBACK;").unwrap();
    drop(writer);
    assert!(!entries(&fixture.archive).iter().any(|name| name.ends_with("-wal") || name.ends_with("-shm")));
    fixture.change();
    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    let after = fixture.known();
    assert_eq!(after.facts, before.facts);
    assert_eq!(after.stand_in_rows, [(1, "Friday".to_owned(), "no vocals".to_owned()), (2, "Saturday".to_owned(), "committed".to_owned())]);
}

/// Stops before each replacement in turn, as a failure and as a crash,
/// until one is not reached because the restore has finished.
#[test]
fn a_restore_that_fails_or_stops_between_any_two_replacements_ends_with_the_library_and_sidecar_as_they_were() {
    let fixture = fixture();
    testing::archive_with(&fixture.location, &fixture.archive, None, Options { stems: true });
    fixture.change();
    let changed = fixture.known();
    let kinds = fixture.sidecar_dir.join(wreckordbox_sidecar::KINDS_FILE);
    let unchanged = |what: &str| {
        assert_eq!(testing::rating(&fixture.location), 1, "{what}");
        assert_eq!(fixture.known(), changed, "{what}");
        assert_eq!(fs::read_to_string(&kinds).unwrap(), "edited", "{what}");
        assert_eq!(fs::read_to_string(fixture.sidecar_dir.join("tag-schema.toml")).unwrap(), "written later", "{what}");
        assert_eq!(fixture.leftovers(), [] as [String; 0], "{what}");
        assert!(!rbl_backup::journal::pending(&fixture.state()), "{what}");
    };
    let mut swaps = 0;
    loop {
        // A failure: the restore puts everything back itself.
        testing::stop_before_swap(swaps, false);
        if fixture.restore(&fixture.archive, Parts::ALL).is_ok() {
            break;
        }
        unchanged(&format!("failure before replacement {swaps}"));
        // A crash: the journal is left, and recovery, run twice as an
        // interrupted recovery would be, puts everything back.
        testing::stop_before_swap(swaps, true);
        assert!(fixture.restore(&fixture.archive, Parts::ALL).is_err());
        assert!(rbl_backup::journal::pending(&fixture.state()));
        if swaps == 3 {
            // The Library's database, WAL and SHM are replaced and nothing
            // of the Sidecar yet: the two disagree until recovery runs.
            assert_eq!(testing::rating(&fixture.location), 4);
            assert_eq!(fs::read_to_string(&kinds).unwrap(), "edited");
        }
        rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
        rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
        unchanged(&format!("crash before replacement {swaps}"));
        // Between the two renames of the same target: it is missing.
        testing::stop_between_renames(swaps, false);
        assert!(fixture.restore(&fixture.archive, Parts::ALL).is_err());
        unchanged(&format!("failure inside replacement {swaps}"));
        testing::stop_between_renames(swaps, true);
        assert!(fixture.restore(&fixture.archive, Parts::ALL).is_err());
        assert!(rbl_backup::journal::pending(&fixture.state()));
        if swaps == 0 {
            assert!(!fixture.location.master_db.exists());
        }
        rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
        rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
        unchanged(&format!("crash inside replacement {swaps}"));
        swaps += 1;
    }
    // The last run went through: the database, its WAL and SHM, the two
    // trees, the Sidecar's entries.
    assert!(swaps > 6, "{swaps}");
    assert_eq!(testing::rating(&fixture.location), 4);
    assert_eq!(fs::read_to_string(&kinds).unwrap(), KINDS);
}

/// A backup holding `sidecar.db` with its `meta.schema_version` set to
/// `version`; the archive's own record stays at a version this build reads.
fn with_schema(fixture: &Fixture, version: u32) -> PathBuf {
    let copy = fixture.dir.path().join(format!("schema-{version}.db"));
    fs::copy(fixture.sidecar_dir.join("sidecar.db"), &copy).unwrap();
    let db = rusqlite::Connection::open(&copy).unwrap();
    db.execute("UPDATE meta SET value = ?1 WHERE key = 'schema_version'", [version.to_string()]).unwrap();
    db.execute_batch("PRAGMA journal_mode=DELETE;").unwrap();
    drop(db);
    rewritten(fixture, "sidecar/sidecar.db", &fs::read(&copy).unwrap())
}

#[test]
fn a_backup_whose_sidecar_schema_is_newer_than_this_build_reads_is_refused_before_anything_is_replaced() {
    assert_eq!(rbl_backup::sidecar_part::SCHEMA_SUPPORTED, wreckordbox_sidecar::SCHEMA_VERSION);
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let changed = fixture.known();
    let newer = with_schema(&fixture, wreckordbox_sidecar::SCHEMA_VERSION + 1);
    assert_eq!(Contents::read(&newer).unwrap().unwrap().version, 1);
    let message = refused(&fixture, &newer, &changed);
    assert!(message.contains("newer wreckordbox"), "{message}");
    // The version this build writes restores.
    let same = with_schema(&fixture, wreckordbox_sidecar::SCHEMA_VERSION);
    fixture.restore(&same, Parts::ALL).unwrap();
    assert_eq!(testing::rating(&fixture.location), 4);
}

#[test]
fn a_databases_companion_files_in_a_backup_are_refused_at_any_depth() {
    let fixture = fixture();
    let nested = fixture.sidecar_dir.join("recipes/index.db");
    rusqlite::Connection::open(&nested).unwrap().execute_batch("CREATE TABLE recipe (name TEXT);").unwrap();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let changed = fixture.known();
    for companion in ["sidecar/sidecar.db-wal", "sidecar/sidecar.db-shm", "sidecar/sidecar.db-journal", "sidecar/recipes/index.db-wal", "sidecar/recipes/index.db-journal"] {
        let bad = fixture.tampered(&[(companion, b"pages")]);
        let message = refused(&fixture, &bad, &changed);
        assert!(message.contains("companion"), "{companion}: {message}");
    }
    // Recorded before databases were listed: found by what the base is.
    let mut record = Contents::read(&fixture.archive).unwrap().unwrap();
    record.databases.clear();
    let unlisted = rewritten(&fixture, "sidecar.json", &serde_json::to_vec(&record).unwrap());
    fs::copy(&unlisted, &fixture.archive).unwrap();
    let bad = fixture.tampered(&[("sidecar/recipes/index.db-wal", b"pages")]);
    let message = refused(&fixture, &bad, &changed);
    assert!(message.contains("companion"), "{message}");
    // A file of the DJ's whose name only ends like one travels.
    fs::write(fixture.sidecar_dir.join("recipes/notes.txt"), "a").unwrap();
    fs::write(fixture.sidecar_dir.join("recipes/notes.txt-journal"), "b").unwrap();
    testing::archive(&fixture.location, &fixture.archive, None);
    assert!(entries(&fixture.archive).contains(&"sidecar/recipes/notes.txt-journal".to_owned()));
    fixture.restore(&fixture.archive, Parts::ALL).unwrap();
    assert_eq!(fs::read_to_string(fixture.sidecar_dir.join("recipes/notes.txt-journal")).unwrap(), "b");
}

#[test]
fn what_a_restore_owes_is_kept_in_its_journal_after_the_files_are_in_place_until_it_is_discharged() {
    let fixture = fixture();
    testing::archive(&fixture.location, &fixture.archive, None);
    fixture.change();
    let owe = serde_json::json!({ "ledger": "restore" });
    let restore_owing = |owed: serde_json::Value| {
        rbl_backup::restore::restore_owing(
            &Request { archive: &fixture.archive, parts: Parts::ALL, location: &fixture.location, state_dir: &fixture.state(), sizes: None },
            Some(owed),
            &|| Ok(()),
            &mut |_, _, _, _| Ok(()),
        )
    };
    restore_owing(owe.clone()).unwrap();
    assert_eq!(testing::rating(&fixture.location), 4);
    assert_eq!(fixture.leftovers(), [] as [String; 0]);
    assert_eq!(rbl_backup::journal::owed(&fixture.state()).unwrap(), Some(owe.clone()));
    // Recovery keeps it; a new restore is refused until it is done.
    rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
    assert_eq!(rbl_backup::journal::owed(&fixture.state()).unwrap(), Some(owe.clone()));
    let error = fixture.restore(&fixture.archive, Parts::ALL).unwrap_err().to_string();
    assert!(error.contains("not been recorded"), "{error}");
    rbl_backup::journal::update_owed(&fixture.state(), serde_json::json!({ "ledger": "restore", "id": 7 })).unwrap();
    assert_eq!(rbl_backup::journal::owed(&fixture.state()).unwrap().unwrap()["id"], 7);
    rbl_backup::journal::discharge(&fixture.state()).unwrap();
    assert!(!rbl_backup::journal::pending(&fixture.state()));

    // Stopped between replacements: rolled back, nothing is owed.
    fixture.change();
    testing::stop_before_swap(2, true);
    assert!(restore_owing(owe.clone()).is_err());
    assert_eq!(rbl_backup::journal::owed(&fixture.state()).unwrap(), None);
    rbl_backup::journal::recover(&fixture.state(), &fixture.location).unwrap();
    assert!(!rbl_backup::journal::pending(&fixture.state()));
    assert_eq!(testing::rating(&fixture.location), 1);
}
