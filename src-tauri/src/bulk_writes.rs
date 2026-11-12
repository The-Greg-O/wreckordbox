//! Window operation lifetimes, separate from the guarded Library write path.
use crate::error::{AppError, ErrorKind};
use crate::{state::AppState, AppResult};
use rbl_library::bulk::{Kind, Operation};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub(crate) struct Operations {
    next: u64,
    open: HashMap<String, (Kind, std::path::PathBuf, Operation)>,
}
#[tauri::command]
pub async fn begin_ledger_operation(
    state: tauri::State<'_, Arc<AppState>>,
    kind: String,
) -> AppResult<String> {
    let kind = match kind.as_str() {
        "analysis" => Kind::Analysis,
        "import" => Kind::Import,
        _ => {
            return Err(AppError::new(
                ErrorKind::Malformed,
                "Unknown Ledger operation.",
            ))
        }
    };
    let location = state.location()?;
    let mut operations = state.bulk_operations.lock();
    if operations.open.len() >= 32 {
        return Err(AppError::new(
            ErrorKind::Internal,
            "Too many unfinished Library operations.",
        ));
    }
    operations.next += 1;
    let token = operations.next.to_string();
    operations.open.insert(
        token.clone(),
        (kind, location.master_db, Operation::new(state.core(), kind)),
    );
    Ok(token)
}
#[tauri::command]
pub async fn finish_ledger_operation(
    state: tauri::State<'_, Arc<AppState>>,
    operation: String,
) -> AppResult<Option<String>> {
    let (_, _, operation) = state
        .bulk_operations
        .lock()
        .open
        .remove(&operation)
        .ok_or_else(|| AppError::new(ErrorKind::NotFound, "That Library operation has ended."))?;
    crate::commands::blocking("finish_ledger_operation", move || Ok(operation.finish())).await
}

pub(crate) struct Recording {
    pub operation: Operation,
    owned: bool,
}
impl Recording {
    pub fn get(state: &AppState, token: Option<&str>, kind: Kind) -> AppResult<Self> {
        let operation = if let Some(token) = token {
            let location = state.location()?;
            let operations = state.bulk_operations.lock();
            let (found, library, operation) = operations.open.get(token).ok_or_else(|| {
                AppError::new(ErrorKind::NotFound, "That Library operation has ended.")
            })?;
            if *found != kind || *library != location.master_db {
                return Err(AppError::new(
                    ErrorKind::Malformed,
                    "That operation belongs to another Library or kind of work.",
                ));
            }
            operation.clone()
        } else {
            Operation::new(state.core(), kind)
        };
        Ok(Self {
            operation,
            owned: token.is_none(),
        })
    }
    pub fn finish<T>(&self, result: AppResult<T>) -> AppResult<(T, Option<String>)> {
        let warning = if self.owned {
            self.operation.finish()
        } else {
            None
        };
        match result {
            Ok(value) => Ok((value, warning)),
            Err(mut error) => {
                if let Some(warning) = warning {
                    error.message.push_str(" ");
                    error.message.push_str(&warning);
                }
                Err(error)
            }
        }
    }
}
