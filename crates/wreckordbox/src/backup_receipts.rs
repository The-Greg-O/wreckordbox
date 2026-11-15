//! `backup receipts [--record]`: the Ledger receipts writes wreckordbox
//! could not record when they were made (`wreckordbox_service::ledger_receipts`).

use std::fmt::Write as _;

use serde_json::json;
use wreckordbox_service::{codes, ledger_receipts, ServiceError};

/// What `backup receipts` answers.
pub fn listed(pending: &[ledger_receipts::Pending], json: bool) -> String {
    if json {
        json!({ "receipts": pending }).to_string()
    } else {
        pending_text(pending)
    }
}

/// What `backup receipts --record` answers: an error while any receipt
/// is kept unrecorded.
pub fn recorded(done: &ledger_receipts::Recorded, json: bool) -> Result<String, ServiceError> {
    if !done.unrecorded.is_empty() {
        let mut message = format!(
            "{} receipt(s) recorded; {} could not be and are kept:",
            done.recorded.len(),
            done.unrecorded.len()
        );
        for item in &done.unrecorded {
            let _ = write!(
                message,
                " {} ({})",
                item.path.display(),
                item.error.as_deref().unwrap_or("unknown")
            );
        }
        return Err(ServiceError::new(codes::RESTORE_UNRECORDED, message));
    }
    Ok(if json {
        json!(done).to_string()
    } else if done.recorded.is_empty() {
        "No receipts to record.".to_owned()
    } else {
        done.recorded
            .iter()
            .map(|r| {
                format!(
                    "Recorded {} as Ledger entry {} in Run {}.",
                    r.operation, r.entry, r.run
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn pending_text(pending: &[ledger_receipts::Pending]) -> String {
    if pending.is_empty() {
        return "No receipts.".to_owned();
    }
    pending
        .iter()
        .map(|item| match (&item.receipt, &item.error) {
            (Some(r), None) => format!(
                "{}  {}  {}  tool:{}",
                r.operation, r.kind, r.outcome, r.source
            ),
            (_, error) => format!(
                "{}  cannot be recorded: {}",
                item.path.display(),
                error.as_deref().unwrap_or("unknown")
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
