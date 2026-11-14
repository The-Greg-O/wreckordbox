//! An explicit DJ receipt precedes deletion; its first entry creates its Run.
use super::{bulk_spill as spill, commands::SPILLS_DISCARD, spill_commands::inspect};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use wreckordbox_sidecar::{
    ledger::{Action, NewEntry, Store},
    RunId, Sidecar, Source, SourceKind,
};

#[derive(Serialize, Deserialize)]
struct Part {
    id: i64,
    payload: Value,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    source: Source,
    id: i64,
    // Older receipts already own a Run. New ones create it with the first entry.
    run: Option<RunId>,
    at: i64,
    payload: Value,
    #[serde(default)]
    more: Vec<Part>,
}
pub(super) fn discard(
    dir: &Path,
    sidecar: &Sidecar,
    id: &str,
    reason: &str,
    source: &Source,
) -> Result<Value, String> {
    let marker = dir.join("discard.json");
    let receipt: Receipt = if marker.exists() {
        spill::read(&marker)?
    } else {
        for path in spill::files(dir)? {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name != "manifest.json"
                && name != "manifest.tmp"
                && !(name.starts_with("batch-") && path.extension().is_some_and(|e| e == "json"))
            {
                return Err(format!(
                    "Cannot name accepted targets in {}; files were kept",
                    path.display()
                ));
            }
        }
        let first = inspect(dir, 0, 128)?;
        if first["pending"]["errors"]
            .as_array()
            .is_none_or(|errors| !errors.is_empty())
        {
            return Err(format!(
                "Cannot name every discarded target; files were kept: {}",
                first["pending"]["errors"]
            ));
        }
        let total = first["pending"]["total"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("Invalid spill count")?;
        let mut lease = wreckordbox_sidecar::bulk::OrderLease::default();
        let first_id = sidecar
            .reserve_bulk_ids(&mut lease, 1)
            .map_err(|e| e.to_string())?[0];
        let mut more = Vec::new();
        for offset in (128..total).step_by(128) {
            let entry = sidecar
                .reserve_bulk_ids(&mut lease, 1)
                .map_err(|e| e.to_string())?[0];
            more.push(Part {
                id: entry,
                payload: json!({"spill":id,"reason":reason,"discarded":inspect(dir,offset,128)?}),
            });
        }
        let receipt = Receipt {
            source: source.clone(),
            id: first_id,
            run: None,
            at: super::now(),
            payload: json!({"spill":id,"reason":reason,"discarded":first}),
            more,
        };
        spill::write(&marker, &receipt)?;
        receipt
    };
    if receipt.source.kind != SourceKind::Dj {
        return Err("A discard receipt must retain the explicit DJ source".into());
    }
    let command =
        json!({"name":SPILLS_DISCARD,"args":{"id":id,"reason":receipt.payload["reason"]}});
    let mut run = receipt.run;
    for (entry_id, payload) in std::iter::once((receipt.id, &receipt.payload))
        .chain(receipt.more.iter().map(|p| (p.id, &p.payload)))
    {
        let entry = NewEntry {
            id: Some(entry_id),
            run: run.unwrap_or(RunId(0)),
            store: Store::Sidecar,
            source: &receipt.source,
            at: Some(receipt.at),
            action: Action::Write,
            kind: "ledger_spill_discard",
            label: "Discard accepted-write Ledger records",
            command: Some(&command),
            reverses: None,
            part_of: (entry_id != receipt.id).then_some(receipt.id),
            reversible: false,
            format: 1,
            payload: Some(payload),
        };
        sidecar
            .append_recovered_following(
                &entry,
                &wreckordbox_sidecar::dj_facts::NativeFacts::default(),
                false,
                &wreckordbox_sidecar::mutant_follow::Follow::default(),
                run.is_none()
                    .then_some("Discard accepted-write Ledger spill"),
            )
            .map_err(|e| e.to_string())?;
        run = Some(
            sidecar
                .ledger_entry(entry_id)
                .map_err(|e| e.to_string())?
                .ok_or("Discard entry missing after commit")?
                .run,
        );
    }
    let run = run.ok_or("Discard Run missing")?;
    sidecar
        .finish_run(run, "explicitly discarded by the DJ")
        .map_err(|e| e.to_string())?;
    // Only after every bounded receipt entry is durable may a byte be removed.
    for path in spill::files(dir)? {
        if path != marker {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::remove_dir(dir).map_err(|e| e.to_string())?;
    if let Some(parent) = dir.parent() {
        std::fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(json!({"id":id,"discarded":true,"entry":receipt.id,"run":run}))
}
