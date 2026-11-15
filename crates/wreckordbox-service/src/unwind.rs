//! Whole-Run Unwind. Planning reads only; applying holds the same edit gate
//! through comparison, the one recorded edit, and refresh.
use crate::{
    library::OpenLibrary,
    protocol::{LibraryChanged, Request, ServiceError},
    stems::{sidecar_error, Stems},
    unwind_values::{self, Part},
};
use rbl_library::{history::LibraryEdit, Touched};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use wreckordbox_sidecar::{
    ledger::{Action, Entry},
    unwind::{FactChange, Visibility},
    RunId, Sidecar,
};

pub const PLAN: &str = "runs.unwind.plan";
pub const APPLY: &str = "runs.unwind";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    id: i64,
    #[serde(default)]
    force: bool,
}

struct Planned {
    report: Value,
    edit: LibraryEdit,
}

pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &Stems,
    keeper: &crate::ledger::Keeper,
    request: &Request,
) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
    let args: Args = serde_json::from_value(request.args.clone())
        .map_err(|e| ServiceError::new("bad_request", e.to_string()))?;
    let apply = request.command == APPLY;
    library.steady(|| {
        if apply && library.protection().protect_library { return Err(crate::protection::refusal()); }
        // Preserve recovery failures instead of presenting an incomplete Run.
        let mut shown = request.clone();
        shown.command = crate::ledger::commands::RUNS_SHOW.into();
        shown.args = json!({"id":args.id,"limit":1});
        let evidence = crate::ledger::answer(keeper, stems, &shown)?;
        if evidence["pending_recording"]["total"].as_u64().unwrap_or(0)>0 || !evidence["pending_recording"]["error"].is_null() || !evidence["recovery"]["error"].is_null() {
            return Err(ServiceError::new("unwind_pending_recording", format!("Run {} has unresolved Ledger recording; inspect runs show and runs spills before Unwind. {evidence}",args.id)));
        }
        let planned = stems.with_sidecar_wait(|sidecar| {
            let sidecar = sidecar.ok_or_else(||ServiceError::new("run_not_found",format!("No Run {}",args.id)))?;
            library.with_db(|db| plan(sidecar, db.connection(), &args))
        })??;
        if !apply || planned.report["reversed"] == 0 {
            let mut report = planned.report;
            report["dry_run"] = json!(!apply);
            return Ok((report,None));
        }
        let mut report = planned.report;
        let edit = planned.edit;
        let ((), changed) = library.recorded_edit(Touched::Tracks,"Unwind Run",move |writer| {
            edit.check(writer,false)?;
            edit.apply(writer,false)?;
            Ok(((),edit))
        })?;
        report["dry_run"] = json!(false);
        // The Ledger may have accepted the write into a spill. Never hide that.
        report["ledger_entry"] = json!(library.latest_ledger_entry());
        Ok((report,Some(changed)))
    })
}

fn plan(
    sidecar: &Sidecar,
    db: &rusqlite::Connection,
    args: &Args,
) -> Result<Planned, ServiceError> {
    let run = sidecar
        .run(RunId(args.id))
        .map_err(|e| sidecar_error(&e))?
        .ok_or_else(|| ServiceError::new("run_not_found", format!("No Run {}", args.id)))?;
    if run.finished_at.is_none() {
        return Err(ServiceError::new(
            "run_open",
            "Finish the Run before unwinding it; more changes can still arrive.",
        ));
    }
    let mut entries = Vec::new();
    loop {
        let (page, total) = sidecar
            .run_entries(run.id, 1000, entries.len())
            .map_err(|e| sidecar_error(&e))?;
        entries.extend(page);
        if entries.len() >= total {
            break;
        }
    }
    let mut virtual_native: BTreeMap<String, Part> = BTreeMap::new();
    let mut virtual_facts: BTreeMap<i64, FactChange> = BTreeMap::new();
    let mut reports = Vec::new();
    let mut reversed = Vec::new();
    let mut reversed_native = Vec::new();
    for entry in entries.into_iter().rev() {
        let mut report = json!({"id":entry.id,"store":entry.store,"kind":entry.kind,"status":"cannot_reverse","values":[],"reason":null});
        if sidecar
            .was_unwound(entry.id, entry.fact.is_some())
            .map_err(|e| sidecar_error(&e))?
            || (entry.fact.is_none()
                && sidecar
                    .undone_outside_run(&entry)
                    .map_err(|e| sidecar_error(&e))?)
        {
            report["status"] = json!("already_reversed");
            reports.push(report);
            continue;
        }
        let raw = entry
            .payload
            .as_ref()
            .and_then(|p| unwind_values::parts(&entry.kind, p));
        if !entry.reversible || (entry.fact.is_none() && (entry.format != 1 || raw.is_none())) {
            report["reason"] = json!(format!(
                "Cannot reverse kind {}: no supported durable inverse (format {}).",
                entry.kind, entry.format
            ));
            reports.push(report);
            continue;
        }
        let mut parts = raw.unwrap_or_default();
        if entry.action == Action::Undo {
            for part in &mut parts {
                std::mem::swap(&mut part.before, &mut part.after);
            }
        }
        let mut conflicts = false;
        let mut missing = false;
        let mut values = Vec::new();
        for part in &mut parts {
            let current = if let Some(now) = virtual_native.get(&part.key) {
                Some(now.after.clone())
            } else {
                unwind_values::current(db, part)
                    .map_err(|e| ServiceError::new("service_failed", e.to_string()))?
            };
            missing |= current.is_none();
            conflicts |= current.as_ref() != Some(&part.after);
            values.push(json!({"subject":part.key,"before":part.before,"after":part.after,"current":current}));
            let target = part.before.clone();
            part.before = current.unwrap_or(Value::Null);
            part.after = target;
        }
        if let Some(fact) = &entry.fact {
            let current = sidecar
                .facts(&fact.track_id, &fact.kind)
                .map_err(|e| sidecar_error(&e))?
                .into_iter()
                .filter(|f| f.source == fact.source && f.is_live())
                .collect::<Vec<_>>();
            values.push(json!({"subject":format!("Fact {} {}",fact.track_id,fact.kind),"after":fact,"current":current,"before":entry.replaced}));
        }
        let (fact_targets, fact_conflict) =
            fact_targets(sidecar, &entry, &virtual_facts, &reversed_native)?;
        conflicts |= fact_conflict;
        let mut fact_changes = Vec::new();
        for target in fact_targets {
            let current = match virtual_facts.get(&target.id) {
                Some(change) => change.after.clone(),
                None => sidecar
                    .fact_visibility(target.id)
                    .map_err(|e| sidecar_error(&e))?
                    .ok_or_else(|| {
                        ServiceError::new("unwind_changed", "A recorded Fact no longer exists")
                    })?,
            };
            values.push(
                json!({"subject":format!("Fact {}",target.id),"current":current,"restore":target,"fact":sidecar.fact(target.id).map_err(|e|sidecar_error(&e))?}),
            );
            fact_changes.push(FactChange {
                before: current,
                after: target,
            });
        }
        report["values"] = json!(values);
        if missing || (conflicts && !args.force) {
            report["status"] = json!("changed_since");
            report["reason"] = json!(if missing {
                "The native row is missing; kept even with force."
            } else {
                "Changed since the Run; kept."
            });
        } else {
            report["status"] = json!("will_reverse");
            report["forced"] = json!(conflicts);
            for part in parts {
                virtual_native
                    .entry(part.key.clone())
                    .and_modify(|old| old.after = part.after.clone())
                    .or_insert(part);
            }
            for change in fact_changes {
                virtual_facts
                    .entry(change.before.id)
                    .and_modify(|old| old.after = change.after.clone())
                    .or_insert(change);
            }
            if entry.fact.is_none() {
                reversed_native.push(entry.id);
            }
            reversed.push(json!({"id":entry.id,"fact":entry.fact.is_some()}));
        }
        reports.push(report);
    }
    let edits = virtual_native
        .values()
        .filter(|p| p.before != p.after)
        .map(unwind_values::edit)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            ServiceError::new("unwind_invalid", "Could not rebuild an Unwind payload")
        })?;
    let fact_changes: Vec<_> = virtual_facts
        .into_values()
        .filter(|f| f.before != f.after)
        .collect();
    let count = reversed.len();
    let metadata = json!({"original_run":args.id,"reverses":reversed,"fact_changes":fact_changes});
    Ok(Planned {
        report: json!({"original_run":args.id,"dry_run":true,"reversed":count,"entries":reports}),
        edit: LibraryEdit::Unwind(rbl_library::unwind::Unwind { edits, metadata }),
    })
}

fn fact_targets(
    sidecar: &Sidecar,
    entry: &Entry,
    virtuals: &BTreeMap<i64, FactChange>,
    reversed: &[i64],
) -> Result<(Vec<Visibility>, bool), ServiceError> {
    let visible = |id| -> Result<Option<Visibility>, ServiceError> {
        if let Some(change) = virtuals.get(&id) {
            return Ok(Some(change.after.clone()));
        }
        sidecar.fact_visibility(id).map_err(|e| sidecar_error(&e))
    };
    if let Some(fact) = &entry.fact {
        let current = visible(fact.id)?;
        let conflict = current.as_ref().is_none_or(|v| v.superseded_at.is_some());
        let mut targets = vec![Visibility {
            id: fact.id,
            superseded_at: Some(now()),
            superseded_by: None,
        }];
        if conflict {
            for live in sidecar
                .facts(&fact.track_id, &fact.kind)
                .map_err(|e| sidecar_error(&e))?
                .into_iter()
                .filter(|f| f.source == fact.source && f.is_live() && f.id != fact.id)
            {
                targets.push(Visibility {
                    id: live.id,
                    superseded_at: Some(now()),
                    superseded_by: None,
                });
            }
        }
        if let Some(previous) = &entry.replaced {
            targets.push(Visibility {
                id: previous.id,
                superseded_at: None,
                superseded_by: None,
            });
        }
        return Ok((targets, conflict));
    }
    if entry.kind == "unwind" {
        let changes: Vec<FactChange> = serde_json::from_value(
            entry
                .payload
                .as_ref()
                .map_or(Value::Null, |p| p["fact_changes"].clone()),
        )
        .map_err(|e| ServiceError::new("unwind_invalid", e.to_string()))?;
        let mut conflict = false;
        let mut targets = Vec::new();
        let ids: Vec<_> = changes.iter().map(|change| change.before.id).collect();
        for change in changes {
            let (expected, target) = if entry.action == Action::Undo {
                (change.before, change.after)
            } else {
                (change.after, change.before)
            };
            conflict |= visible(expected.id)?.as_ref() != Some(&expected);
            targets.push(target);
        }
        for fact in sidecar
            .unwind_related_facts(&ids)
            .map_err(|e| sidecar_error(&e))?
        {
            if !ids.contains(&fact.id)
                && visible(fact.id)?.is_some_and(|v| v.superseded_at.is_none())
            {
                conflict = true;
                targets.push(Visibility {
                    id: fact.id,
                    superseded_at: Some(now()),
                    superseded_by: None,
                });
            }
        }
        return Ok((targets, conflict));
    }
    let mut targets = sidecar
        .native_unwind_targets(entry)
        .map_err(|e| sidecar_error(&e))?;
    let changed = !targets.is_empty()
        && sidecar
            .native_effects_changed(entry.id, reversed, virtuals)
            .map_err(|e| sidecar_error(&e))?;
    if changed {
        for fact in sidecar
            .native_unwind_facts(entry.id)
            .map_err(|e| sidecar_error(&e))?
        {
            if visible(fact.id)?.is_some_and(|v| v.superseded_at.is_none())
                && !targets.iter().any(|target| target.id == fact.id)
            {
                targets.push(Visibility {
                    id: fact.id,
                    superseded_at: Some(now()),
                    superseded_by: None,
                });
            }
        }
    }
    Ok((targets, changed))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
