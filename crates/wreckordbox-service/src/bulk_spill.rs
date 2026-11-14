//! Bounded, idempotent bulk spill files. Only startup/write-side code replays;
//! query helpers below only read. Each chunk is one recording batch.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use wreckordbox_sidecar::{
    ledger::{Action, NewEntry, Store},
    RunId, Sidecar, Source,
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub id: Option<i64>,
    pub at: i64,
    pub value: Value,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub intent: Option<i64>,
    pub run: Option<RunId>,
    pub source: Source,
    pub description: String,
    pub kind: String,
    pub complete: bool,
}
impl Manifest {
    pub fn append(&self, sidecar: &Sidecar, records: &[Record]) -> Result<(), String> {
        let run = self.run.ok_or("bulk Run is not available")?;
        let descriptions: Vec<_> = records
            .iter()
            .map(|record| {
                super::player_record::describe(&self.kind, &self.description, &record.value)
            })
            .collect();
        let commands: Vec<_> = descriptions
            .iter()
            .map(|description| {
                let mut command = json!({"name":"bulk.write","args":{"kind":self.kind}});
                // The apply-time drift check reads command targets as well as the
                // plan's barrier reader. Both must see the same bounded tracks.
                if let Some(tracks) = description.payload.get("tracks") {
                    command["args"]["tracks"] = tracks.clone();
                }
                command
            })
            .collect();
        let entries: Vec<_> = records
            .iter()
            .zip(&descriptions)
            .zip(&commands)
            .map(|((record, description), command)| NewEntry {
                id: record.id,
                run,
                store: Store::Library,
                source: &self.source,
                at: Some(record.at),
                action: if description.reversible {
                    Action::Edit
                } else {
                    Action::Write
                },
                kind: &description.kind,
                label: &description.label,
                command: Some(command),
                reverses: None,
                part_of: None,
                reversible: description.reversible,
                format: 1,
                payload: Some(&description.payload),
            })
            .collect();
        sidecar.append_bulk(&entries).map_err(|e| e.to_string())
    }
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}
fn local(path: &Path) -> Result<(), String> {
    if wreckordbox_stick_protection::stick_root(path).is_some() {
        return Err("Ledger recovery storage is on a Stick; named results must remain in memory until local storage is available".into());
    }
    Ok(())
}
fn write(path: &Path, value: &impl Serialize) -> Result<(), String> {
    local(path)?;
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(super) fn save(
    dir: &Path,
    manifest: &Manifest,
    index: u64,
    records: &[Record],
) -> Result<(), String> {
    local(dir)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let header = dir.join("manifest.json");
    if !header.exists() {
        write(&header, manifest)?;
    }
    write(&dir.join(format!("batch-{index:020}.json")), &records)
}
fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    files.sort();
    Ok(files)
}
enum Header {
    Ready(Manifest),
    Empty,
    Orphaned(String),
}

/// Only a missing header is special. Unreadable or malformed manifests retain
/// the normal recovery failure. Never mistake a batch (even a .tmp batch) for
/// disposable metadata; conservatively keep unknown files too.
fn header(dir: &Path) -> Result<Header, String> {
    let path = dir.join("manifest.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Header::Ready)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if files(dir)?
                .iter()
                .any(|p| p.file_name().is_none_or(|n| n != "manifest.tmp"))
            {
                Ok(Header::Orphaned(format!(
                    "Bulk spill {} has no manifest.json; files that may contain accepted writes are kept for recovery",
                    dir.display()
                )))
            } else {
                Ok(Header::Empty)
            }
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn remove_empty(dir: &Path) -> Result<(), String> {
    local(dir)?;
    for result in [
        std::fs::remove_file(dir.join("manifest.tmp")),
        std::fs::remove_dir(dir),
    ] {
        if let Err(error) = result {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("{}: {error}", dir.display()));
            }
        }
    }
    Ok(())
}

pub(super) fn mark_complete(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let file = dir.join("manifest.json");
    let mut manifest = match header(dir)? {
        Header::Ready(manifest) => manifest,
        Header::Empty => return remove_empty(dir),
        Header::Orphaned(error) => return Err(error),
    };
    manifest.complete = true;
    write(&file, &manifest)
}
pub(super) fn replay(
    dir: &Path,
    sidecar: &Sidecar,
    mut before: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    local(dir)?;
    let mut manifest = match header(dir)? {
        Header::Ready(manifest) => manifest,
        Header::Empty => return remove_empty(dir),
        Header::Orphaned(error) => return Err(error),
    };
    let header = dir.join("manifest.json");
    if manifest.run.is_none() {
        manifest.run = Some(
            sidecar
                .begin_bulk_run(manifest.intent, &manifest.source, &manifest.description)
                .map_err(|e| e.to_string())?,
        );
        // Persist the Run before any entry: an interrupted replay keeps one Run.
        write(&header, &manifest)?;
    }
    for path in files(dir)?.into_iter().filter(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("batch-"))
            && p.extension().is_some_and(|e| e == "json")
    }) {
        let mut records: Vec<Record> = read(&path)?;
        if records.len() > 128 {
            return Err("Oversized bulk spill batch; left untouched".into());
        }
        // A reservation failure at acceptance must not make retries duplicate
        // entries: persist replacement ids before publishing the recovered batch.
        let missing = records.iter().filter(|r| r.id.is_none()).count();
        if missing > 0 {
            let ids = sidecar
                .reserve_bulk_ids(
                    &mut wreckordbox_sidecar::bulk::OrderLease::default(),
                    missing,
                )
                .map_err(|e| e.to_string())?;
            for (record, id) in records.iter_mut().filter(|r| r.id.is_none()).zip(ids) {
                record.id = Some(id);
            }
            write(&path, &records)?;
        }
        before()?;
        manifest.append(sidecar, &records)?;
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    if manifest.complete {
        if let Some(intent) = manifest.intent {
            sidecar
                .settle_intents(&[intent])
                .map_err(|e| e.to_string())?;
        }
        if let Some(run) = manifest.run {
            sidecar
                .finish_run(run, "completed after recording recovery")
                .map_err(|e| e.to_string())?;
        }
    }
    std::fs::remove_file(header).map_err(|e| e.to_string())?;
    std::fs::remove_dir(dir).map_err(|e| e.to_string())?;
    Ok(())
}
pub(super) fn exists(root: &Path) -> bool {
    root.join("ledger-bulk").is_dir()
}
/// Completed/startup work with no manifest cannot be attributed safely. Keep
/// it and return a diagnostic separately so unrelated writes can still proceed.
pub(super) fn recover(dir: &Path, sidecar: &Sidecar) -> Result<Option<String>, String> {
    if let Header::Orphaned(error) = header(dir)? {
        return Ok(Some(error));
    }
    replay(dir, sidecar, || Ok(()))?;
    Ok(None)
}
pub(super) fn replay_all(root: &Path, sidecar: &Sidecar) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for dir in files(&root.join("ledger-bulk"))? {
        if dir.is_dir() {
            if let Some(error) = recover(&dir, sidecar)? {
                errors.push(error);
            }
        }
    }
    Ok(errors)
}
/// Named accepted writes waiting on disk. A query never replays or cleans up.
pub(super) fn pending(
    root: &Path,
    run: Option<i64>,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let mut records = Vec::new();
    let mut total = 0;
    for dir in files(&root.join("ledger-bulk"))? {
        if !dir.is_dir() {
            continue;
        }
        let manifest = match header(&dir)? {
            Header::Ready(manifest) => manifest,
            Header::Empty => continue,
            Header::Orphaned(error) => return Err(error),
        };
        if manifest.run.map(|id| id.0) != run {
            continue;
        }
        for path in files(&dir)?.into_iter().filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("batch-"))
                && p.extension().is_some_and(|e| e == "json")
        }) {
            for record in read::<Vec<Record>>(&path)? {
                if total >= offset && records.len() < limit {
                    records.push(json!({"id":record.id,"at":record.at,"result":record.value}));
                }
                total += 1;
            }
        }
    }
    Ok(
        json!({"records":records,"total":total,"offset":offset,"next_offset":if offset+records.len()<total {Some(offset+records.len())}else{None}}),
    )
}

pub(super) fn bind_run(dir: &Path, current: &Manifest) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let path = dir.join("manifest.json");
    let mut manifest: Manifest = read(&path)?;
    if manifest.run.is_none() && current.run.is_some() {
        manifest.run = current.run;
        write(&path, &manifest)?;
    }
    Ok(())
}
