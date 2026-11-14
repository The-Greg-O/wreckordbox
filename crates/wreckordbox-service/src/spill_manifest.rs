//! A supplied recovery manifest must repeat authority already in the Ledger.
use super::bulk_spill::Manifest;
use serde_json::Value;
use std::{io::Write, path::Path};
use wreckordbox_sidecar::{RunId, Sidecar, Source};

pub(super) fn validate(sidecar: &Sidecar, manifest: &Manifest) -> Result<(), String> {
    let id = manifest
        .intent
        .ok_or("Recovery requires the original intent's Ledger id")?;
    let (source, run, command): (Source, Option<RunId>, Option<Value>) = if let Some(intent) =
        sidecar
            .open_intents()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|i| i.id == id)
    {
        (intent.source, intent.run, intent.command)
    } else {
        let entry = sidecar
            .ledger_entry(id)
            .map_err(|e| e.to_string())?
            .filter(|e| e.kind == super::UNCONFIRMED)
            .ok_or("The original open intent or unconfirmed entry is not in the Ledger")?;
        (entry.source, Some(entry.run), entry.command)
    };
    let command = command.unwrap_or(Value::Null);
    if source != manifest.source
        || run != manifest.run
        || command["name"] != "bulk.write"
        || command["args"]["kind"] != manifest.kind
    {
        return Err("The recovery manifest must exactly match the Ledger's original source, Run and operation kind".into());
    }
    Ok(())
}

/// Keep the bytes outside the replay queue, even after successful replay removes
/// its folder. Explicit recovery alone creates this archive; reads never do.
pub(super) fn preserve_corrupt(dir: &Path) -> Result<(), String> {
    let path = dir.join("manifest.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let root = dir
        .parent()
        .and_then(Path::parent)
        .ok_or("Missing recovery root")?;
    let archive = root
        .join("ledger-bulk-evidence")
        .join(dir.file_name().ok_or("Missing spill id")?);
    super::bulk_spill::local(&archive)?;
    std::fs::create_dir_all(&archive).map_err(|e| e.to_string())?;
    let saved = archive.join("manifest.json");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&saved)
    {
        Ok(mut file) => {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&saved).map_err(|e| e.to_string())? != bytes {
                return Err("A different corrupt manifest is already archived; neither copy was overwritten".into());
            }
        }
        Err(e) => return Err(e.to_string()),
    }
    for path in [&archive, &root.join("ledger-bulk-evidence"), root] {
        std::fs::File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
