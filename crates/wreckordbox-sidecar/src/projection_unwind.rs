//! How Projection reads an Unwind entry (wbx-23): its native track edits,
//! and the edits it reverses, each traced to the edit at its origin.
use crate::projection::{FieldState, Owner};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// The `track` edits inside an Unwind payload, `None` when it does not read.
pub(crate) fn track_edits(payload: Option<&str>) -> Option<Vec<Value>> {
    let payload: Value = serde_json::from_str(payload?).ok()?;
    let mut edits = Vec::new();
    for edit in payload["edits"].as_array()? {
        if edit["kind"].as_str()? == "track" {
            edits.extend(edit["payload"]["edits"].as_array()?.iter().cloned());
        }
    }
    Some(edits)
}

/// The Ledger edits an Unwind reverses (Fact entries are another namespace
/// and are left out). `None` when it does not read.
pub(crate) fn reversed<O>(
    payload: Option<&str>,
    steps: &HashMap<i64, (String, Option<i64>, O)>,
) -> Option<HashSet<i64>> {
    let payload: Value = serde_json::from_str(payload?).ok()?;
    let mut origins = HashSet::new();
    for item in payload["reverses"].as_array()? {
        if item["fact"] == true {
            continue;
        }
        // Reversing a window Undo or Redo re-applies an edit; its owner is
        // not known here, so such a field is left Unknown.
        let id = item["id"].as_i64()?;
        if steps
            .get(&id)
            .is_some_and(|(action, _, _)| action == "edit")
        {
            origins.insert(id);
        }
    }
    Some(origins)
}

/// The `track_tags` parts of an Unwind payload as one `track_tags` payload,
/// `None` when it does not read.
pub(crate) fn tag_payload(payload: Option<&str>) -> Option<String> {
    let payload: Value = serde_json::from_str(payload?).ok()?;
    let mut tags = Vec::new();
    for edit in payload["edits"].as_array()? {
        if edit["kind"].as_str()? == "track_tags" {
            tags.extend(edit["payload"]["tags"].as_array()?.iter().cloned());
        }
    }
    Some(json!({ "tags": tags }).to_string())
}

/// One field's state after an Unwind (`edit`) entry `id`: back to its state
/// before the oldest edit the Unwind reverses, so that edit's owner no longer
/// owns it, or Unknown when the field's history does not show one. Pushes
/// the Unwind itself, so a window Undo of it returns to `current`.
pub(crate) fn restore(
    stack: &mut Vec<(i64, Option<FieldState>)>,
    reversed: Option<&HashSet<i64>>,
    current: Option<FieldState>,
    id: i64,
) -> FieldState {
    let at = reversed.and_then(|ids| stack.iter().position(|(step, _)| ids.contains(step)));
    let restored = match at {
        Some(at) => {
            let before = stack[at].1.clone();
            stack.truncate(at);
            before.map_or(
                FieldState {
                    owner: Owner::Original,
                    value: Value::Null,
                    entry: id,
                },
                |state| FieldState { entry: id, ..state },
            )
        }
        None => FieldState {
            owner: Owner::Unknown,
            value: Value::Null,
            entry: id,
        },
    };
    stack.push((id, current));
    restored
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::ledger::{Action, NewEntry, Store};
    use crate::projection::Owner;
    use crate::{Sidecar, Source, SourceKind};
    use serde_json::{json, Value};

    fn append(sidecar: &Sidecar, source: &Source, kind: &str, payload: &Value) -> i64 {
        let run = sidecar.begin_run(source, "test").unwrap();
        sidecar
            .append(&NewEntry {
                id: None,
                run,
                store: Store::Library,
                source,
                at: None,
                action: Action::Edit,
                kind,
                label: "test",
                command: None,
                reverses: None,
                part_of: None,
                reversible: true,
                format: 1,
                payload: Some(payload),
            })
            .unwrap()
    }

    /// An unwound MyTag assignment is no longer the unwound agent's, so a
    /// Projection may fill it; an assignment the Unwind left stays theirs.
    #[test]
    fn an_unwound_assignment_goes_back_to_its_state_before_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let agent = Source::new(SourceKind::Agent, "bad");
        let on = |track: &str| json!({"track": track, "before": [], "after": ["t"]});
        let mistaken = append(
            &sidecar,
            &agent,
            "track_tags",
            &json!({"tags": [on("1")], "comments": []}),
        );
        append(
            &sidecar,
            &agent,
            "track_tags",
            &json!({"tags": [on("2")], "comments": []}),
        );
        let undone = json!({"track": "1", "before": ["t"], "after": []});
        append(
            &sidecar,
            &agent,
            "unwind",
            &json!({
                "edits": [{"kind": "track_tags", "payload": {"tags": [undone], "comments": []}}],
                "original_run": 1,
                "reverses": [{"id": mistaken, "fact": false}],
                "fact_changes": [],
            }),
        );
        let found = sidecar.tag_provenance().unwrap();
        let owner = |track: &str| {
            found.assignments[&(track.to_owned(), "t".to_owned())]
                .owner
                .clone()
        };
        assert_eq!(owner("1"), Owner::Original);
        assert_eq!(owner("2"), Owner::Other(agent.clone()));
        assert!(found.barriers.is_empty(), "{:?}", found.barriers);
    }
}
