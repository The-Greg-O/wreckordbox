//! Unwind a whole Run through the Library service.
use serde_json::{json,Value};
use wreckordbox_service::{Client,ServiceError};

pub fn run(client: &mut Client, as_json: bool, id: i64, dry_run: bool, force: bool) -> Result<String,ServiceError> {
    let command = if dry_run { wreckordbox_service::unwind::PLAN } else { wreckordbox_service::unwind::APPLY };
    let report: Value = client.call(command,json!({"id":id,"force":force}))?;
    if as_json { return Ok(report.to_string()); }
    let mut lines = vec![format!("{} Run {id}: {} entries {}.",if dry_run {"Unwind plan for"} else {"Unwind of"},report["reversed"],if dry_run {"will be reversed"} else {"reversed"})];
    for entry in report["entries"].as_array().into_iter().flatten() {
        let status = match entry["status"].as_str().unwrap_or("") {
            "will_reverse" if !dry_run => "reversed",
            "will_reverse" => "will be reversed",
            "changed_since" => "changed since (kept)",
            "already_reversed" => "already reversed (kept)",
            _ => "cannot be reversed",
        };
        lines.push(format!("Entry {} ({}): {status}{}",entry["id"],entry["kind"].as_str().unwrap_or("?"),if entry["forced"]==true {" (forced)"} else {""}));
        if let Some(reason)=entry["reason"].as_str() { lines.push(format!("  {reason}")); }
        for value in entry["values"].as_array().into_iter().flatten() { lines.push(format!("  {value}")); }
    }
    Ok(lines.join("\n"))
}
