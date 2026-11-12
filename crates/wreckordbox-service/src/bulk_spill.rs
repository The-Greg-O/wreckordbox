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
        let payloads: Vec<_> = records
            .iter()
            .map(|record| {
                json!({
                    "records":[record.value],"not_reversible_reason":super::bulk::REASON,
                })
            })
            .collect();
        let command = json!({"name":"bulk.write","args":{"kind":self.kind}});
        let label = format!(
            "{}. Cannot reverse: {}",
            self.description,
            super::bulk::REASON
        );
        let entries: Vec<_> = records
            .iter()
            .zip(&payloads)
            .map(|(record, payload)| NewEntry {
                id: record.id,
                run,
                store: Store::Library,
                source: &self.source,
                at: Some(record.at),
                action: Action::Write,
                kind: &self.kind,
                label: &label,
                command: Some(&command),
                reverses: None,
                part_of: None,
                reversible: false,
                format: 1,
                payload: Some(payload),
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
pub(super) fn mark_complete(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let file = dir.join("manifest.json");
    let mut manifest: Manifest = read(&file)?;
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
    let header = dir.join("manifest.json");
    let mut manifest: Manifest = read(&header)?;
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
pub(super) fn replay_all(root: &Path, sidecar: &Sidecar) -> Result<(), String> {
    for dir in files(&root.join("ledger-bulk"))? {
        if dir.is_dir() {
            replay(&dir, sidecar, || Ok(()))?;
        }
    }
    Ok(())
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
        let manifest: Manifest = read(&dir.join("manifest.json"))?;
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
