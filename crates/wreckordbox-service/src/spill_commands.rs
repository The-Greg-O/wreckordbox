//! Explicit management of local accepted-write spills. Reads never repair them.
use super::commands::{SPILLS_DISCARD, SPILLS_LIST, SPILLS_REPLAY, SPILLS_SHOW};
use super::{bulk_spill as spill, spill_lease::Lease, Keeper};
use crate::{
    protocol::{codes, Request, ServiceError},
    stems::Stems,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use wreckordbox_sidecar::SourceKind;

pub(super) fn answers(command: &str) -> bool {
    matches!(
        command,
        SPILLS_LIST | SPILLS_SHOW | SPILLS_REPLAY | SPILLS_DISCARD
    )
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "used directly as a map_err adapter"
)]
fn bad(why: impl ToString) -> ServiceError {
    ServiceError::new(codes::BAD_REQUEST, why.to_string())
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "used directly as a map_err adapter"
)]
fn failed(why: impl ToString) -> ServiceError {
    ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why.to_string())
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    id: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    manifest: Option<spill::Manifest>,
    source: Option<crate::facts::SourceArg>,
    reason: Option<String>,
}
fn directory(root: &Path, id: &str) -> Result<PathBuf, ServiceError> {
    if id.is_empty()
        || !matches!(
            Path::new(id).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
    {
        return Err(bad("Use a spill id from runs.spills.list"));
    }
    let dir = root.join("ledger-bulk").join(id);
    validate_directory(&dir).map_err(failed)?;
    Ok(dir)
}
fn validate_directory(dir: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Spill must be a local directory, not a symlink".into());
    }
    for path in spill::files(dir)? {
        if !std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err(format!(
                "Spill contains a non-regular file: {}",
                path.display()
            ));
        }
    }
    spill::local(dir)
}

/// Summary pages are separate from accepted-result pages; corrupt batches stay visible.
pub(super) fn inventory(root: &Path, offset: usize, limit: usize) -> Result<Value, String> {
    let dirs = spill::files(&root.join("ledger-bulk"))?;
    let total = dirs.len();
    let spills: Vec<Value> = dirs
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|dir| {
            let id = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            match inspect(&dir, 0, 0) {
                Ok(mut value) => {
                    value["id"] = json!(id);
                    value
                }
                Err(error) => json!({"id":id,"error":error}),
            }
        })
        .collect();
    Ok(
        json!({"spills":spills,"total":total,"offset":offset,"next_offset":super::next_offset(offset,spills.len(),total)}),
    )
}
pub(super) fn inspect(dir: &Path, offset: usize, limit: usize) -> Result<Value, String> {
    validate_directory(dir)?;
    let (manifest, error) = match spill::header(dir) {
        Ok(spill::Header::Ready(m)) => (Some(m), None),
        Ok(spill::Header::Empty) => (None, None),
        Ok(spill::Header::Orphaned(e)) | Err(e) => (None, Some(e)),
    };
    let mut files = Vec::new();
    let mut records = Vec::new();
    let mut total = 0;
    let mut errors = Vec::new();
    for path in spill::files(dir)? {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        files.push(json!({"name":name,"bytes":bytes}));
        if name.starts_with("batch-") && path.extension().is_some_and(|ext| ext == "json") {
            match spill::read_batch(&path) {
                Ok(batch) => {
                    for record in batch {
                        if total >= offset && records.len() < limit {
                            records
                                .push(json!({"id":record.id,"at":record.at,"result":record.value}));
                        }
                        total += 1;
                    }
                }
                Err(error) => errors.push(error),
            }
        }
    }
    Ok(
        json!({"manifest":manifest,"error":error,"files":files,"discard_pending":dir.join("discard.json").exists(),
        "pending":{"records":records,"total":total,"offset":offset,"next_offset":super::next_offset(offset,records.len(),total),"errors":errors}}),
    )
}

pub(super) fn answer(
    keeper: &Keeper,
    stems: &Stems,
    request: &Request,
) -> Result<Value, ServiceError> {
    let args: Args = serde_json::from_value(if request.args.is_null() {
        json!({})
    } else {
        request.args.clone()
    })
    .map_err(bad)?;
    let root = stems.bulk_recovery_dir();
    let offset = args.offset.unwrap_or(0);
    let limit = args.limit.unwrap_or(50).min(1000);
    if request.command == SPILLS_LIST {
        return inventory(&root, offset, limit).map_err(failed);
    }
    let id = args
        .id
        .as_deref()
        .ok_or_else(|| bad("A spill id is required"))?;
    let dir = directory(&root, id)?;
    if request.command == SPILLS_SHOW {
        return inspect(&dir, offset, limit).map_err(failed);
    }
    if request.command == SPILLS_DISCARD
        && args
            .source
            .as_ref()
            .is_none_or(|s| s.kind != SourceKind::Dj)
    {
        return Err(bad("Discarding accepted-write records requires the DJ's explicit source; agents and tools cannot discard them"));
    }
    let _lease = Lease::take(&dir).map_err(failed)?;
    let _access = stems.bulk_access().map_err(failed)?;
    let sidecar = stems.sidecar()?;
    let answer = if request.command == SPILLS_REPLAY {
        if dir.join("discard.json").exists() {
            return Err(bad(
                "A discard is pending. Inspect it and explicitly retry discard.",
            ));
        }
        if let Some(mut manifest) = args.manifest {
            if matches!(spill::header(&dir), Ok(spill::Header::Ready(_))) {
                return Err(bad("An existing manifest cannot be replaced"));
            }
            if manifest.source.name.trim().is_empty()
                || manifest.description.trim().is_empty()
                || !matches!(
                    manifest.kind.as_str(),
                    "analysis" | "import" | "link" | "round_trip"
                )
            {
                return Err(bad(
                    "Recovery needs the original source, description and operation kind",
                ));
            }
            let tool = match manifest.kind.as_str() {
                "analysis" => Some("wreckordbox-analysis"),
                "link" => Some("link"),
                "round_trip" => Some("round-trip"),
                _ => None,
            };
            if tool.is_some_and(|name| {
                manifest.source.kind != SourceKind::Tool || manifest.source.name != name
            }) {
                return Err(bad(
                    "Analysis and player writes must retain their original tool source",
                ));
            }
            super::spill_manifest::validate(&sidecar, &manifest).map_err(bad)?;
            super::spill_binding::authorize(&dir, &sidecar, &manifest, None).map_err(bad)?;
            super::spill_manifest::preserve_corrupt(&dir).map_err(failed)?;
            manifest.complete = true;
            spill::write(&dir.join("manifest.json"), &manifest).map_err(failed)?;
        }
        spill::mark_complete(&dir).map_err(failed)?;
        spill::replay(&dir, &sidecar, None, || Ok(())).map_err(failed)?;
        json!({"id":id,"replayed":true})
    } else {
        let reason = args
            .reason
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| bad("Explain why these accepted-write records are being discarded"))?;
        let source = crate::facts::source(
            args.source
                .as_ref()
                .ok_or_else(|| bad("An explicit DJ source is required"))?,
        )?;
        super::spill_discard::discard(&dir, &sidecar, id, reason, &source).map_err(failed)?
    };
    // State ownership is short; no Keeper mutex spans disk or Sidecar I/O.
    let mut state = keeper.state();
    state.bulk_waiting.retain(|p| p != &dir);
    let named = dir.display().to_string();
    state.spill_refusals.retain(|e| !e.contains(&named));
    if state
        .recovery_error
        .as_ref()
        .is_some_and(|e| e.message.contains(&dir.display().to_string()))
    {
        state.recovery_error = None;
    }
    Ok(answer)
}
/// A spill recovery refused stays on disk; the report says why.
pub(super) fn include_refusals(keeper: &Keeper, root: &Path, recovery: &mut Value) {
    let Some(spills) = recovery["spills"].as_array_mut() else {
        return;
    };
    let state = keeper.state();
    for spill in spills.iter_mut().filter(|s| s["error"].is_null()) {
        let Some(id) = spill["id"].as_str() else {
            continue;
        };
        let named = root.join("ledger-bulk").join(id).display().to_string();
        if let Some(refusal) = state
            .spill_refusals
            .iter()
            .rev()
            .find(|e| e.contains(&format!("{named} ")))
        {
            spill["error"] = json!(refusal);
        }
    }
}
/// In-memory append failures are accepted writes too. Copy only this page while
/// owning state; inspection neither settles the intent nor writes the spill.
pub(super) fn include_ordinary(
    keeper: &Keeper,
    run: Option<i64>,
    offset: usize,
    limit: usize,
    pending: &mut Value,
) {
    let Some(bulk_total) = pending["total"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
    else {
        return;
    };
    let Some(records) = pending["records"].as_array_mut() else {
        return;
    };
    let state = keeper.state();
    let matching = || state.unwritten.iter().filter(|entry| entry.run == run);
    let total = bulk_total.saturating_add(matching().count());
    let skip = offset.saturating_sub(bulk_total);
    for entry in matching()
        .skip(skip)
        .take(limit.saturating_sub(records.len()))
    {
        records.push(json!({"id":entry.id,"at":entry.at,"result":{"kind":entry.kind,"label":entry.label,"source":entry.source,"payload":entry.payload,"command":entry.command}}));
    }
    let shown = records.len();
    pending["total"] = json!(total);
    pending["next_offset"] = json!(super::next_offset(offset, shown, total));
}
