//! Window operation lifetimes, separate from the guarded Library write path.
use crate::error::{AppError, ErrorKind};
use crate::{state::AppState, AppResult};
use rbl_library::bulk::{Kind, Operation};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
const IDLE: Duration = Duration::from_secs(60);
struct Entry {
    kind: Kind,
    library: std::path::PathBuf,
    operation: Operation,
    used: parking_lot::Mutex<Instant>,
}
impl Entry {
    fn idle(entry: &Arc<Self>) -> bool {
        Arc::strong_count(entry) == 1 && entry.operation.idle()
    }
}
#[derive(Default)]
pub(crate) struct Operations {
    next: u64,
    reaper: bool,
    open: HashMap<String, Arc<Entry>>,
}
impl Operations {
    fn expired(&mut self, now: Instant) -> Vec<Arc<Entry>> {
        let keys: Vec<_> = self
            .open
            .iter()
            .filter(|(_, e)| Entry::idle(e) && now.duration_since(*e.used.lock()) >= IDLE)
            .map(|(k, _)| k.clone())
            .collect();
        keys.into_iter()
            .filter_map(|k| self.open.remove(&k))
            .collect()
    }
}
fn finish_removed(entries: Vec<Arc<Entry>>) {
    for entry in entries {
        if let Some(warning) = entry.operation.finish() {
            tracing::warn!(%warning,"expired Ledger operation");
        }
    }
}
fn start_reaper(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let Some(state) = weak.upgrade() else { return };
        let expired = state.bulk_operations.lock().expired(Instant::now());
        finish_removed(expired);
    });
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
    let state = Arc::clone(state.inner());
    crate::commands::blocking("begin_ledger_operation", move || {
        let location = state.location()?;
        let (token, removed, start) = {
            let mut operations = state.bulk_operations.lock();
            let mut removed = operations.expired(Instant::now());
            if operations.open.len() >= 32 {
                let oldest = operations
                    .open
                    .iter()
                    .filter(|(_, e)| Entry::idle(e))
                    .min_by_key(|(_, e)| *e.used.lock())
                    .map(|(k, _)| k.clone());
                if let Some(key) = oldest {
                    if let Some(entry) = operations.open.remove(&key) {
                        removed.push(entry);
                    }
                }
            }
            operations.next += 1;
            let token = operations.next.to_string();
            // All slots busy: a missing token falls back to a per-call Run. A
            // recording capacity limit must never refuse an accepted Library write.
            if operations.open.len() < 32 {
                operations.open.insert(
                    token.clone(),
                    Arc::new(Entry {
                        kind,
                        library: location.master_db,
                        operation: Operation::new(state.core(), kind),
                        used: parking_lot::Mutex::new(Instant::now()),
                    }),
                );
            }
            let start = !operations.reaper;
            operations.reaper = true;
            (token, removed, start)
        };
        finish_removed(removed);
        if start {
            start_reaper(&state);
        }
        Ok(token)
    })
    .await
}
#[tauri::command]
pub async fn finish_ledger_operation(
    state: tauri::State<'_, Arc<AppState>>,
    operation: String,
) -> AppResult<Option<String>> {
    let entry = state.bulk_operations.lock().open.remove(&operation);
    crate::commands::blocking("finish_ledger_operation",move||Ok(entry.map_or_else(
        ||Some("The Ledger operation expired or exceeded capacity; later writes were recorded in per-call Runs.".into()),
        |entry|entry.operation.finish_wait()
    ))).await
}
pub(crate) struct Recording {
    pub operation: Operation,
    owned: bool,
    entry: Option<Arc<Entry>>,
    warning: Option<String>,
}
impl Recording {
    pub fn get(state: &AppState, token: Option<&str>, kind: Kind) -> AppResult<Self> {
        let entry = if let Some(token) = token {
            let location = state.location()?;
            let operations = state.bulk_operations.lock();
            if let Some(entry) = operations.open.get(token) {
                if entry.kind != kind || entry.library != location.master_db {
                    return Err(AppError::new(
                        ErrorKind::Malformed,
                        "That operation belongs to another Library or kind of work.",
                    ));
                }
                *entry.used.lock() = Instant::now();
                Some(entry.clone())
            } else {
                None
            }
        } else {
            None
        };
        let owned = entry.is_none();
        let operation = entry.as_ref().map_or_else(
            || Operation::new(state.core(), kind),
            |e| e.operation.clone(),
        );
        let warning = (owned && token.is_some()).then(|| {
            "The Ledger operation expired or exceeded capacity; this write has its own Run.".into()
        });
        Ok(Self {
            operation,
            owned,
            entry,
            warning,
        })
    }
    pub fn finish<T>(&self, result: AppResult<T>) -> AppResult<(T, Option<String>)> {
        let warning = join_warning(
            self.warning.clone(),
            if self.owned {
                self.operation.finish()
            } else {
                None
            },
        );
        match result {
            Ok(value) => Ok((value, warning)),
            Err(mut error) => {
                if let Some(warning) = warning {
                    error.message.push(' ');
                    error.message.push_str(&warning);
                }
                Err(error)
            }
        }
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        if let Some(entry) = &self.entry {
            *entry.used.lock() = Instant::now();
        }
    }
}
pub(crate) fn join_warning(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn idle_expiry_flushes_accepted_results_but_does_not_expire_an_active_client() {
        let dir = tempfile::tempdir().unwrap();
        let location = rbl_db::fixture::build(
            dir.path(),
            rbl_db::fixture::Shape {
                tracks: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let core = Arc::new(rbl_library::LibraryCore::new(dir.path().join("state")));
        core.set_protection(false).unwrap();
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (index, _) = rbl_index::load(&db).unwrap();
        core.set_library(
            &rbl_library::NoHost,
            index,
            false,
            db.schema().db_version,
            0,
            location.clone(),
        );
        let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(rbl_library::NoHost);
        let service = wreckordbox_service::Service::host(
            &location,
            &wreckordbox_service::Config {
                runtime_dir: dir.path().join("run"),
                idle_exit: None,
                recovery_dir: dir.path().join("state"),
            },
            &core,
            &host,
        )
        .unwrap();
        let entry = Arc::new(Entry {
            kind: Kind::Analysis,
            library: location.master_db,
            operation: Operation::new(&core, Kind::Analysis),
            used: parking_lot::Mutex::new(Instant::now()),
        });
        entry
            .operation
            .run(
                json!({"track":"10000"}),
                || {
                    core.write_then(
                        &rbl_library::NoHost,
                        rbl_library::History::Outside,
                        |w| {
                            w.set_analysis(
                                "10000",
                                &rbl_db::write::AnalysisWrite {
                                    bpm_x100: 12345,
                                    key: None,
                                    analysis_path: "/fixture/ANLZ0000.DAT",
                                    length_sec: None,
                                },
                            )
                        },
                        |_, v| Ok(v),
                    )
                },
                |_| vec![json!({"track":"10000"})],
            )
            .unwrap();
        let mut operations = Operations::default();
        operations.open.insert("one".into(), entry.clone());
        let later = Instant::now() + IDLE + Duration::from_secs(1);
        assert!(
            operations.expired(later).is_empty(),
            "an active client owns a reference"
        );
        drop(entry);
        let removed = operations.expired(later);
        assert_eq!(removed.len(), 1);
        finish_removed(removed);
        let runs = service.call("runs.list", json!({})).unwrap();
        assert_eq!(runs["total"], 1);
        let shown = service
            .call("runs.show", json!({"id":runs["runs"][0]["id"]}))
            .unwrap();
        assert_eq!(
            shown["entries"][0]["payload"]["records"][0]["track"],
            "10000"
        );
        assert!(shown["run"]["finished_at"].is_i64());
    }
}
