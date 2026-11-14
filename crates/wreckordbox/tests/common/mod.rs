//! A throwaway Library, runtime directory and home for one test, and the
//! real `wreckordbox` binary pointed at them.
#![allow(dead_code)]

pub mod barrier;
pub mod catalogue;
pub mod released;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use rbl_db::fixture::{self, Shape};
use rbl_db::write::{TrackField, Writer};
use rbl_db::LibraryLocation;
use wreckordbox_service::{Client, Config};

pub struct Run {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }

    pub fn error_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stderr)
            .unwrap_or_else(|e| panic!("stderr is not JSON ({e}): {}", self.stderr))
    }
}

/// What [`Sandbox::detail_track`] sets. Texts are as the window's Info
/// panel takes them; an empty one is left alone.
#[derive(Default)]
pub struct Details<'a> {
    pub album: &'a str,
    pub genre: &'a str,
    pub label: &'a str,
    pub year: &'a str,
    pub play_count: &'a str,
    pub comment: &'a str,
    /// The colour label as the Library numbers it: 1 Pink, 2 Red, 3 Orange,
    /// 4 Yellow, 5 Green, 6 Aqua, 7 Blue, 8 Purple.
    pub colour_id: Option<u8>,
    /// MyTag identifiers, such as `fixture::MY_TAG_PEAK`.
    pub my_tags: &'a [&'a str],
}

pub struct Sandbox {
    dir: tempfile::TempDir,
    location: LibraryLocation,
    /// Seconds a service started here stays up with no client connected.
    pub idle_exit_secs: u32,
    /// Options for the fake separation engine a service started here runs
    /// (`separation-engine/tests/fake_worker.py`): `--short ROLE`,
    /// `--delay SECONDS`, `--fail CATEGORY`. Set them before the first
    /// command, which is what starts the service.
    pub engine_options: Vec<String>,
    /// The folders standing in for the mounted Sticks a service started here
    /// sees (each must carry the stand-in marker to count as a Stick). Empty
    /// (the default): none; a sandbox's service never looks at the
    /// machine's real mounts. Set before the first command.
    pub sticks: Vec<PathBuf>,
    /// Services to stop when the test ends.
    pids: Mutex<Vec<u32>>,
}

impl Sandbox {
    /// A fixture Library of 40 tracks.
    pub fn new() -> Self {
        Self::with_shape(Shape::default())
    }

    /// A fixture Library with no tracks at all.
    pub fn empty() -> Self {
        Self::with_shape(Shape {
            tracks: 0,
            playlists: 0,
            tracks_per_playlist: 0,
            history_sessions: 0,
            ..Shape::default()
        })
    }

    /// A fixture Library on a machine where nothing has said whether
    /// Library Protection is on: no window has run and nobody has turned it
    /// off. Every other sandbox starts with protection off, as on the
    /// machine of a DJ who edits.
    pub fn untouched() -> Self {
        let sandbox = Self::new();
        std::fs::remove_file(sandbox.protection_file()).unwrap();
        sandbox
    }

    /// Makes every Stick report `bytes` free, instead of what its folder's
    /// file system has (test mode only; read by the service each time).
    pub fn set_stick_free_bytes(&self, bytes: u64) {
        std::fs::write(
            self.state_dir()
                .join(wreckordbox_service::stick_load::TEST_FREE_BYTES_FILE),
            bytes.to_string(),
        )
        .unwrap();
    }

    /// Makes every Stick's file system store files in clusters of `bytes`
    /// (test mode only; read by the service each time).
    pub fn set_stick_cluster_bytes(&self, bytes: u64) {
        std::fs::write(
            self.state_dir()
                .join(wreckordbox_service::stick_load::TEST_CLUSTER_BYTES_FILE),
            bytes.to_string(),
        )
        .unwrap();
    }

    /// The folder the window and the service share their state in.
    pub fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// Where Library Protection is kept for the service to read.
    pub fn protection_file(&self) -> PathBuf {
        self.state_dir().join("library-protection.json")
    }

    /// Turns Library Protection on or off as any writer records it: its
    /// turn under the lock, its own temporary file, a higher revision.
    pub fn protect(&self, on: bool) {
        wreckordbox_service::protection::store(&self.state_dir(), on).unwrap();
    }

    pub fn with_shape(shape: Shape) -> Self {
        let sandbox = Self::with_shape_untouched(shape);
        sandbox.protect(false);
        sandbox
    }

    fn with_shape_untouched(shape: Shape) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        let location = fixture::build(&library, shape).unwrap();
        fixture::write_options_json(
            &dir.path().join("options.json"),
            location.master_db.to_str().unwrap(),
            &location.passphrase,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        Self {
            dir,
            location,
            idle_exit_secs: 5,
            engine_options: Vec::new(),
            sticks: Vec::new(),
            pids: Mutex::new(Vec::new()),
        }
    }

    /// Changes the fixture Library with upstream's own writer, as the window
    /// would. Call it before anything starts a service: the service is the
    /// Library's only writer once it is up.
    pub fn edit(&self, change: impl FnOnce(&mut Writer)) {
        let mut writer =
            Writer::open(self.location.clone(), self.dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        change(&mut writer);
    }

    /// Gives the `index`th fixture track a title, an artist, a key and a
    /// rating.
    pub fn describe_track(&self, index: usize, title: &str, artist: &str, key: &str, stars: u8) {
        let id = fixture::track_id(index);
        self.edit(|writer| {
            writer.set_field(&id, TrackField::Title, title).unwrap();
            writer.set_field(&id, TrackField::Artist, artist).unwrap();
            writer.ensure_detected_key(key).unwrap();
            writer.set_field(&id, TrackField::Key, key).unwrap();
            writer.set_rating(&id, stars).unwrap();
        });
    }

    /// Gives the `index`th fixture track the rest of what a DJ fills in.
    /// A field left at its default is not touched.
    pub fn detail_track(&self, index: usize, details: &Details) {
        let id = fixture::track_id(index);
        self.edit(|writer| {
            for (field, value) in [
                (TrackField::Album, details.album),
                (TrackField::Genre, details.genre),
                (TrackField::Label, details.label),
                (TrackField::Year, details.year),
                (TrackField::PlayCount, details.play_count),
            ] {
                if !value.is_empty() {
                    writer.set_field(&id, field, value).unwrap();
                }
            }
            if !details.comment.is_empty() {
                writer.set_comment(&id, details.comment).unwrap();
            }
            if let Some(colour) = details.colour_id {
                writer.set_color(&id, Some(&colour.to_string())).unwrap();
            }
            if !details.my_tags.is_empty() {
                let tags: Vec<String> = details
                    .my_tags
                    .iter()
                    .map(|tag| (*tag).to_owned())
                    .collect();
                writer.set_my_tags(&id, &tags).unwrap();
            }
        });
    }

    /// Adds a MyTag to the fixture's one category, Situation, and answers
    /// its identifier.
    pub fn create_my_tag(&self, name: &str) -> String {
        let mut id = String::new();
        self.edit(|writer| {
            let tag = writer.create_my_tag(fixture::MY_TAG_CATEGORY).unwrap();
            writer.rename_my_tag(&tag.id, name).unwrap();
            id = tag.id;
        });
        id
    }

    /// Gives the fixture the four MyTag categories every rekordbox Library
    /// has, and answers their identifiers in order: the fixture's own
    /// Situation, then Mood, Sound and Sub-genre, each empty.
    ///
    /// The Library's writer cannot make a category (rekordbox fixes them),
    /// so the three rows are written as the fixture builder writes its one:
    /// `Attribute` 1 under `root`, numbered on after it.
    pub fn four_categories(&self) -> [String; 4] {
        let added = [
            ("700101", "Mood"),
            ("700102", "Sound"),
            ("700103", "Sub-genre"),
        ];
        self.edit(|writer| {
            for (place, (id, name)) in added.iter().enumerate() {
                writer
                    .library()
                    .connection()
                    .execute_batch(&format!(
                        "INSERT INTO djmdMyTag
                            (ID, Seq, Name, Attribute, ParentID, UUID,
                             rb_data_status, rb_local_data_status, rb_local_deleted,
                             rb_local_synced, usn, rb_local_usn, created_at, updated_at)
                         SELECT '{id}', {seq}, '{name}', 1, 'root', 'fixture-mytag-{id}',
                                0, 0, 0, 0, NULL, rb_local_usn, created_at, updated_at
                         FROM djmdMyTag WHERE ID = '{first}'",
                        seq = place + 2,
                        first = fixture::MY_TAG_CATEGORY,
                    ))
                    .unwrap();
            }
        });
        [
            fixture::MY_TAG_CATEGORY.to_owned(),
            "700101".to_owned(),
            "700102".to_owned(),
            "700103".to_owned(),
        ]
    }

    /// Names colour labels as the DJ names them in rekordbox's preferences:
    /// `(colour, name)`, the colour numbered as [`Details::colour_id`]. The
    /// fixture's colour table starts empty, so a colour not named here goes
    /// by its own name.
    pub fn name_colours(&self, names: &[(u8, &str)]) {
        self.edit(|writer| {
            for (colour, name) in names {
                writer
                    .library()
                    .connection()
                    .execute(
                        "INSERT INTO djmdColor
                            (ID, ColorCode, SortKey, Commnt, UUID, created_at, updated_at)
                         VALUES (?1, 0, ?1, ?2, 'fixture-colour-' || ?1,
                                 '2026-01-01', '2026-01-01')",
                        (colour.to_string(), name),
                    )
                    .unwrap();
            }
        });
    }

    /// The DJ's own Prep colours: Buy on Pink, Problem on Red, Sample on
    /// Yellow, Clean on Green and Delete on Purple.
    pub fn dj_prep_colours(&self) {
        self.name_colours(&[
            (1, "Buy"),
            (2, "Problem"),
            (3, "Orange"),
            (4, "Sample"),
            (5, "Clean"),
            (6, "Aqua"),
            (7, "Blue"),
            (8, "Delete"),
        ]);
    }

    /// Adds a MyTag called `name` to the end of `category` and answers its
    /// identifier.
    pub fn create_my_tag_in(&self, category: &str, name: &str) -> String {
        let mut id = String::new();
        self.edit(|writer| {
            let tag = writer.create_my_tag(category).unwrap();
            writer.rename_my_tag(&tag.id, name).unwrap();
            id = tag.id;
        });
        id
    }

    /// Adds a playlist holding the fixture tracks at `indexes`, in that
    /// order, and answers its identifier.
    pub fn create_playlist(&self, name: &str, indexes: &[usize]) -> String {
        let mut id = String::new();
        self.edit(|writer| {
            id = writer.create_playlist(name, "root").unwrap();
            let tracks: Vec<String> = indexes
                .iter()
                .map(|&index| fixture::track_id(index))
                .collect();
            writer.add_tracks(&id, &tracks).unwrap();
        });
        id
    }

    /// Adds an intelligent playlist of the tracks whose genre contains
    /// `genre`, and answers its identifier.
    pub fn create_genre_playlist(&self, name: &str, genre: &str) -> String {
        let mut id = String::new();
        self.edit(|writer| {
            id = writer
                .create_smart_playlist(name, "root", |playlist| {
                    format!(
                        "<NODE Id=\"{playlist}\" LogicalOperator=\"1\" AutomaticUpdate=\"0\">\
                         <CONDITION PropertyName=\"genre\" Operator=\"8\" ValueUnit=\"\" \
                         ValueLeft=\"{genre}\" ValueRight=\"\"/></NODE>"
                    )
                })
                .unwrap();
        });
        id
    }

    /// Imports a silent audio file of `seconds` as rekordbox imports one:
    /// added today, not yet analysed. Answers the new track's identifier.
    pub fn import_track(&self, file_name: &str, seconds: u32) -> String {
        let path = self.dir.path().join("audio").join(file_name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_wav(&path, seconds);
        let mut id = String::new();
        self.edit(|writer| id = writer.import_file(&path).unwrap());
        id
    }

    pub fn master_db(&self) -> &Path {
        &self.location.master_db
    }

    pub fn location(&self) -> &LibraryLocation {
        &self.location
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The private runtime directory the service's socket goes in.
    pub fn runtime_dir(&self) -> PathBuf {
        self.dir.path().join("run")
    }

    /// The Sidecar of this sandbox's Library: a folder of its own under the
    /// temporary folder, named to the service by `WRECKORDBOX_SIDECAR_DIR`.
    pub fn sidecar_dir(&self) -> PathBuf {
        self.dir.path().join("sidecar")
    }

    /// How many separations the fake engine has been asked for.
    pub fn engine_runs(&self) -> usize {
        std::fs::read_to_string(self.engine_log()).map_or(0, |log| log.lines().count())
    }

    fn engine_log(&self) -> PathBuf {
        self.dir.path().join("engine.log")
    }

    /// The command a service here runs as the separation engine: the
    /// engine's real worker protocol over its fake backend. Never a model.
    fn separation_command(&self) -> String {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../separation-engine/tests/fake_worker.py");
        let mut command = vec![
            "python3".to_owned(),
            fake.to_str().unwrap().to_owned(),
            "--log".to_owned(),
            self.engine_log().to_str().unwrap().to_owned(),
        ];
        command.extend(self.engine_options.iter().cloned());
        serde_json::to_string(&command).unwrap()
    }

    /// What a second client in this sandbox connects with.
    pub fn config(&self) -> Config {
        Config {
            runtime_dir: self.runtime_dir(),
            idle_exit: None,
            recovery_dir: self.state_dir(),
        }
    }

    /// A second client, connected to the service already running here.
    pub fn client(&self) -> Client {
        Client::connect(&self.config().socket_path(&self.location)).unwrap()
    }

    /// The `wreckordbox` binary with this sandbox's environment and nothing
    /// else: no display, no real home, no real runtime directory.
    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wreckordbox"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("RBXPORT_TEST", "1")
            .env("RBXPORT_STATE_DIR", self.state_dir())
            .env("RBXPORT_OPTIONS", self.dir.path().join("options.json"))
            .env("RBXPORT_TEST_FIXTURE", self.dir.path().join("library"))
            .env("WRECKORDBOX_RUNTIME_DIR", self.runtime_dir())
            .env("WRECKORDBOX_SIDECAR_DIR", self.sidecar_dir())
            .env("WRECKORDBOX_SEPARATION_COMMAND", self.separation_command())
            .env(
                "WRECKORDBOX_IDLE_EXIT_SECS",
                self.idle_exit_secs.to_string(),
            )
            .env(
                "RB_LITE_FAKE_VOLUMES",
                std::env::join_paths(&self.sticks).unwrap(),
            )
            .stdin(Stdio::null());
        command
    }

    pub fn cli(&self, args: &[&str]) -> Run {
        self.run(self.command().args(args))
    }

    pub fn run(&self, command: &mut Command) -> Run {
        let output = command.output().unwrap();
        let run = Run {
            status: output.status,
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        };
        self.remember_service(&run);
        run
    }

    /// `status --json`, which must succeed.
    pub fn status(&self) -> serde_json::Value {
        let run = self.cli(&["--json", "status"]);
        assert!(run.status.success(), "{}", run.stderr);
        run.json()
    }

    /// Runs `wreckordbox serve` in the foreground and waits until it answers.
    pub fn serve(&self) -> Child {
        let mut child = self
            .command()
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.pids.lock().unwrap().push(child.id());
        let socket = self.config().socket_path(&self.location);
        let up = (0..400).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(25));
            Client::connect(&socket).is_ok()
        });
        if !up {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the service did not come up");
        }
        child
    }

    fn remember_service(&self, run: &Run) {
        let pid = serde_json::from_str::<serde_json::Value>(&run.stdout)
            .ok()
            .and_then(|status| status.get("pid").and_then(serde_json::Value::as_u64))
            .and_then(|pid| u32::try_from(pid).ok());
        if let Some(pid) = pid {
            self.pids.lock().unwrap().push(pid);
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // A service this sandbox's commands started without saying so.
        if let Ok(mut client) = Client::connect(&self.config().socket_path(&self.location)) {
            if let Ok(status) = client.status() {
                self.pids.lock().unwrap().push(status.pid);
            }
        }
        // Belt and braces: a service also stops by itself when idle.
        for pid in self.pids.lock().unwrap().drain(..) {
            let _ = Command::new("kill")
                .arg(pid.to_string())
                .stderr(Stdio::null())
                .status();
        }
    }
}

pub fn alive(pid: u64) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// A silent 8 kHz mono 16-bit WAV file.
fn write_wav(path: &Path, seconds: u32) {
    let rate: u32 = 8_000;
    let data_len = rate * 2 * seconds;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(44 + data_len as usize, 0);
    std::fs::write(path, out).unwrap();
}

/// A command's text output without the last line saying which Run its
/// changes are in, which every edit prints; the output itself when there
/// is none.
#[allow(dead_code)]
pub fn without_run(stdout: &str) -> String {
    let mut lines: Vec<&str> = stdout.lines().collect();
    if lines.last().is_some_and(|line| line.starts_with("Run ")) {
        lines.pop();
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}
