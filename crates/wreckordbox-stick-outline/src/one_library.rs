//! A Stick's `exportLibrary.db`, opened so that nothing on the Stick can be
//! created, changed or deleted (wbx-45.5).
//!
//! `ExportLibrary::open_read_only` protects the database file itself, but
//! SQLite opens a WAL-mode database's `-wal` with create rights and its
//! `-shm` with `O_RDWR|O_CREAT` whatever the connection's flags, so reading
//! a Stick rekordbox left in WAL mode could create or rebuild those files on
//! it. Here:
//!
//! - with no companion beside the database (`-wal`, `-shm`, `-journal`), it
//!   is opened through SQLite's `immutable` URI parameter, under which SQLite
//!   neither locks nor opens any companion;
//! - otherwise the database and its `-wal` are copied to a temporary folder
//!   off the Stick and the copy is opened normally, so committed rows still
//!   in the `-wal` are read and nothing touches the Stick.
//!
//! The tests snapshot the whole Stick folder around every read, for WAL-mode
//! Sticks with and without a `-wal`. Beside the protected crates (ADR 0003).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

/// Companions SQLite may keep beside a database.
const COMPANIONS: [&str; 3] = ["-wal", "-shm", "-journal"];

fn companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Whether any SQLite companion sits beside `path`.
#[must_use]
pub fn has_companions(path: &Path) -> bool {
    COMPANIONS
        .iter()
        .any(|suffix| companion(path, suffix).exists())
}

/// Applies the cipher settings an `exportLibrary.db` needs, as
/// `rbl_onelibrary` does (the cipher and legacy mode before the key), once
/// SQLCipher's process-wide setup has finished.
///
/// # Errors
///
/// Why the key could not be derived or applied.
pub fn keyed(conn: &Connection) -> Result<(), String> {
    ready()?;
    let passphrase = rbl_onelibrary::key::passphrase().map_err(|e| e.to_string())?;
    conn.pragma_update(None, "cipher", "sqlcipher")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "legacy", 4)
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "key", passphrase)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Waits out SQLCipher's process-wide setup, as `rbl_onelibrary` does: a
/// connection keyed on another thread while it runs fails.
fn ready() -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    static READY: AtomicBool = AtomicBool::new(false);
    if READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let probe = Connection::open_in_memory().map_err(|e| e.to_string())?;
        match probe.pragma_update(None, "key", "ready") {
            Ok(()) => {
                READY.store(true, Ordering::Release);
                return Ok(());
            }
            Err(error) if Instant::now() >= deadline => return Err(error.to_string()),
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

/// An `exportLibrary.db` opened for reading without anything on the Stick
/// being created or written. Holds the off-Stick copy, when one was made,
/// for as long as the connection.
#[derive(Debug)]
pub struct OneLibraryRead {
    conn: Connection,
    _copy: Option<tempfile::TempDir>,
}

impl OneLibraryRead {
    /// Opens the `exportLibrary.db` at `path` (see the module documentation).
    ///
    /// # Errors
    ///
    /// Why it could not be copied, opened, keyed or read.
    pub fn open(path: &Path) -> Result<Self, String> {
        if !has_companions(path) {
            let conn = immutable(path)?;
            // A companion that appeared meanwhile (another program opened
            // the database) means the immutable view may be stale: read a
            // copy instead. The immutable connection created nothing.
            if !has_companions(path) {
                return Ok(Self { conn, _copy: None });
            }
        }
        Self::copied(path)
    }

    /// The database and its `-wal`, copied off the Stick and opened there.
    fn copied(path: &Path) -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let copy = dir.path().join("exportLibrary.db");
        std::fs::copy(path, &copy).map_err(|e| e.to_string())?;
        let wal = companion(path, "-wal");
        if wal.exists() {
            std::fs::copy(&wal, companion(&copy, "-wal")).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open_with_flags(
            &copy,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| e.to_string())?;
        keyed(&conn)?;
        conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            _copy: Some(dir),
        })
    }

    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// `path` opened with SQLite's `immutable` URI parameter: no locks, no
/// companion files opened or created.
fn immutable(path: &Path) -> Result<Connection, String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut uri = String::from("file:");
    for byte in absolute.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(char::from(byte))
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    keyed(&conn)?;
    conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|e| e.to_string())?;
    Ok(conn)
}
