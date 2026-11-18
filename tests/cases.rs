//! The reader and validator against fsm-cli's test cases (`FSM_CASES`, else `../fsm-cli/tests/cases`): the same
//! problems, at the same lines, with the same ids. fsm-cli's own messages about files it can't read (a missing
//! submachine file, one that isn't valid) are its loader's: a submachine is then reported as not found.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use fsm_editor_common::format::read_model;
use fsm_editor_common::model::{VertexType, machine_connection_points};
use fsm_editor_common::validate::{SubmachineInfo, validate};
use serde_json::{Value, json};

fn cases() -> PathBuf {
    std::env::var("FSM_CASES").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("../fsm-cli/tests/cases"))
}

#[test]
fn fsm_cli_cases() {
    let root = cases();
    let mut folders = 0;
    for entry in fs::read_dir(&root).expect("fsm-cli's test cases").flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        folders += 1;
        let expected: serde_json::Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
        let texts: HashMap<&String, String> =
            expected.keys().map(|f| (f, fs::read_to_string(dir.join(f)).unwrap())).collect();
        let read: HashMap<&String, _> = texts.iter().map(|(f, t)| (*f, read_model(t))).collect();
        for (f, list) in &expected {
            let found: Vec<Value> = match &read[f] {
                Err(problems) => problems
                    .iter()
                    .map(|p| json!({ "line": p.line, "id": "", "severity": "error", "message": p.message }))
                    .collect(),
                Ok(r) => {
                    let mut info = HashMap::new();
                    for v in &r.model.vertices {
                        let Some(sub) = v.submachine.as_ref().filter(|s| v.vtype == VertexType::State && !s.is_empty())
                        else {
                            continue;
                        };
                        let i = match read.get(sub) {
                            Some(Ok(m)) => SubmachineInfo {
                                found: true,
                                name: Some(m.model.name.clone()),
                                file: None,
                                points: machine_connection_points(&m.model),
                            },
                            _ => SubmachineInfo::default(),
                        };
                        info.insert(sub.clone(), i);
                    }
                    validate(&r.model, &info)
                        .into_iter()
                        .map(|i| {
                            json!({ "line": r.lines.get(&i.id).copied().unwrap_or(1), "id": i.id, "severity": i.severity, "message": i.message })
                        })
                        .collect()
                }
            };
            let wanted: Vec<Value> = list
                .as_array()
                .unwrap()
                .iter()
                .filter(|i| {
                    let m = i["message"].as_str().unwrap();
                    !m.ends_with(" does not exist or cannot be read.")
                        && !m.ends_with(" is not a valid state machine file.")
                })
                .cloned()
                .collect();
            assert_eq!(found, wanted, "{}/{f}", dir.display());
        }
    }
    assert!(folders >= 4);
}
