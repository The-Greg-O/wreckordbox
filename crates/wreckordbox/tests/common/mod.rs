//! A throwaway Library, runtime directory and home for one test, and the
//! real `wreckordbox` binary pointed at them.
#![allow(dead_code)]

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

pub struct Sandbox {
    dir: tempfile::TempDir,
    location: LibraryLocation,
    /// Seconds a service started here stays up with no client connected.
    pub idle_exit_secs: u32,
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

    pub fn with_shape(shape: Shape) -> Self {
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

    /// What a second client in this sandbox connects with.
    pub fn config(&self) -> Config {
        Config {
            runtime_dir: self.runtime_dir(),
            idle_exit: None,
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
            .env("RBXPORT_OPTIONS", self.dir.path().join("options.json"))
            .env("RBXPORT_TEST_FIXTURE", self.dir.path().join("library"))
            .env("WRECKORDBOX_RUNTIME_DIR", self.runtime_dir())
            .env(
                "WRECKORDBOX_IDLE_EXIT_SECS",
                self.idle_exit_secs.to_string(),
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
