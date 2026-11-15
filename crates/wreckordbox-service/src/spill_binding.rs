//! Every spill is bound to the intent it was accepted under: `intent.json`,
//! written before the spill's first batch and never replaced. A replay whose
//! manifest names an intent must repeat that binding and match the Ledger;
//! the intent must belong to no operation still running and must not have
//! been replayed already from another spill. A spill written before binding
//! replays only when the Ledger holds exactly one matching candidate.
use super::{bulk_spill as spill, bulk_spill::Manifest, spill_manifest};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, PoisonError},
};
use wreckordbox_sidecar::Sidecar;

pub(super) const FILE: &str = "intent.json";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Binding {
    pub intent: Option<i64>,
    /// The intent's recorded time: an intent id alone may repeat in another Library.
    pub at: Option<i64>,
}

/// Before the first batch; a later batch must repeat the same binding.
pub(super) fn bind(dir: &Path, binding: Binding) -> Result<(), String> {
    let path = dir.join(FILE);
    match bound(dir)? {
        Some(existing) if existing == binding => Ok(()),
        Some(_) => Err(format!("{} is bound to another intent", path.display())),
        None => spill::write(&path, &binding),
    }
}
fn bound(dir: &Path) -> Result<Option<Binding>, String> {
    let path = dir.join(FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => spill::read(&path).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The intents of operations still running in this process, by Sidecar.
static LIVE: LazyLock<Mutex<HashSet<(PathBuf, i64)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
pub(super) struct Live((PathBuf, i64));
impl Live {
    pub(super) fn register(sidecar: &Sidecar, intent: i64) -> Self {
        let key = (sidecar.dir().to_path_buf(), intent);
        LIVE.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone());
        Self(key)
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        LIVE.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}
fn live(sidecar: &Sidecar, intent: i64) -> bool {
    LIVE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&(sidecar.dir().to_path_buf(), intent))
}

fn refused(dir: &Path, why: &str) -> String {
    format!(
        "Spill {} was refused: {why}; it is kept for the DJ's inspect or discard",
        dir.display()
    )
}
fn name(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn marker(dir: &Path, intent: i64) -> Result<PathBuf, String> {
    let root = dir
        .parent()
        .and_then(Path::parent)
        .ok_or("Missing recovery root")?;
    Ok(root
        .join("ledger-bulk-replayed")
        .join(format!("{intent}.json")))
}
/// The spill an intent was already replayed from, if any.
fn replayed_from(dir: &Path, intent: i64) -> Result<Option<String>, String> {
    let path = marker(dir, intent)?;
    if !path.exists() {
        return Ok(None);
    }
    let value: serde_json::Value = spill::read(&path)?;
    Ok(Some(value["spill"].as_str().unwrap_or_default().to_owned()))
}

/// `own` is the intent of the operation replaying its own spill.
pub(super) fn authorize(
    dir: &Path,
    sidecar: &Sidecar,
    manifest: &Manifest,
    own: Option<i64>,
) -> Result<(), String> {
    let binding = bound(dir).map_err(|e| refused(dir, &e))?;
    let Some(id) = manifest.intent else {
        return match binding.and_then(|b| b.intent) {
            Some(intent) => Err(refused(
                dir,
                &format!("its manifest names no intent, but it was accepted under intent {intent}"),
            )),
            None => Ok(()),
        };
    };
    let usable = |intent: i64| own == Some(intent) || !live(sidecar, intent);
    if !usable(id) {
        return Err(refused(
            dir,
            &format!("intent {id} belongs to an operation that is still running"),
        ));
    }
    match binding {
        Some(binding) => {
            if binding.intent != Some(id) {
                return Err(refused(
                    dir,
                    &format!(
                        "its manifest names intent {id}, but it was accepted under intent {}",
                        binding
                            .intent
                            .map_or_else(|| "none".to_owned(), |i| i.to_string())
                    ),
                ));
            }
            let at = spill_manifest::validate(sidecar, manifest).map_err(|e| refused(dir, &e))?;
            if binding.at != Some(at) {
                return Err(refused(
                    dir,
                    &format!("intent {id} in this Ledger is not the intent it was accepted under (another Library's spill?)"),
                ));
            }
        }
        None => {
            let mut found = Vec::new();
            for candidate in spill_manifest::candidates(sidecar, manifest, usable)
                .map_err(|e| refused(dir, &e))?
            {
                if replayed_from(dir, candidate)?.is_none_or(|from| from == name(dir)) {
                    found.push(candidate);
                }
            }
            if found != [id] {
                return Err(refused(
                    dir,
                    &format!("it predates intent binding and the Ledger holds {} matching candidates for intent {id}, not exactly that one", found.len()),
                ));
            }
        }
    }
    if let Some(from) = replayed_from(dir, id)? {
        if from != name(dir) {
            return Err(refused(
                dir,
                &format!("intent {id} was already replayed from spill {from}"),
            ));
        }
    }
    Ok(())
}

/// Durably claims the intent for this spill before its first entry; a retry
/// of the same spill keeps its claim.
pub(super) fn claim(dir: &Path, intent: i64) -> Result<(), String> {
    let path = marker(dir, intent)?;
    if let Some(from) = replayed_from(dir, intent)? {
        return if from == name(dir) {
            Ok(())
        } else {
            Err(refused(
                dir,
                &format!("intent {intent} was already replayed from spill {from}"),
            ))
        };
    }
    let parent = path.parent().ok_or("Missing replay marker folder")?;
    spill::local(parent)?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(json!({"spill":name(dir)}).to_string().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::File::open(parent)
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())
}
