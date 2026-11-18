//! Comparing and merging against `fixtures/compare.json`: pairs of machines with what the editors' reference gave
//! (written by the umbrella repo's `parity/common-fixtures.mjs`).

use fsm_editor_common::compare::{compare_models, describe_change, is_model_change};
use fsm_editor_common::merge::{adopt, has_conflict_markers, split_conflicts};
use fsm_editor_common::model::FsmModel;
use serde_json::{Value, json};

/// Numbers as floats (2000 and 2000.0 alike), objects without order.
fn norm(v: &Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), norm(&o[k]))).collect())
        }
        other => other.clone(),
    }
}

/// Where two values differ, as `path: got != want` lines.
fn diff(got: &Value, want: &Value, at: &str) -> String {
    match (got, want) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.iter()
                .filter(|k| a.get(**k) != b.get(**k))
                .map(|k| {
                    diff(a.get(*k).unwrap_or(&Value::Null), b.get(*k).unwrap_or(&Value::Null), &format!("{at}.{k}"))
                })
                .collect::<Vec<_>>()
                .join("")
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => (0..a.len())
            .filter(|&i| a[i] != b[i])
            .map(|i| diff(&a[i], &b[i], &format!("{at}[{i}]")))
            .collect::<Vec<_>>()
            .join(""),
        _ => format!("  {at}: {got} != {want}\n"),
    }
}

#[test]
fn compare_fixture() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/compare.json")).unwrap();
    let mut bad = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let a: FsmModel = serde_json::from_value(case["a"].clone()).unwrap();
        let b: FsmModel = serde_json::from_value(case["b"].clone()).unwrap();
        for (dir, older, newer) in [("ab", &a, &b), ("ba", &b, &a)] {
            let want = &case[dir];
            let changes = compare_models(older, newer);
            if norm(&serde_json::to_value(&changes).unwrap()) != norm(&want["changes"]) {
                bad.push(format!(
                    "{name} {dir}: changes\n  got  {}\n  want {}",
                    serde_json::to_string(&changes).unwrap(),
                    want["changes"]
                ));
                continue;
            }
            for (i, c) in changes.iter().enumerate() {
                let d = json!([describe_change(c, true), describe_change(c, false), is_model_change(c)]);
                if d != want["described"][i] {
                    bad.push(format!("{name} {dir}: described {i}: {d} vs {}", want["described"][i]));
                }
                let mut keys = vec![c.key.clone()];
                keys.extend(c.old_key.clone());
                let got = adopt(older, newer, &keys);
                let got = norm(&serde_json::to_value(&got).unwrap());
                let wanted = norm(&want["adopted"][i]);
                if got != wanted {
                    bad.push(format!("{name} {dir}: adopt {}\n{}", c.key, diff(&got, &wanted, "")));
                }
            }
        }
    }
    for c in fixture["conflicts"].as_array().unwrap() {
        let text = c["text"].as_str().unwrap();
        assert_eq!(json!(has_conflict_markers(text)), c["markers"], "markers {text:?}");
        let split = match split_conflicts(text) {
            Ok(s) => serde_json::to_value(s).unwrap(),
            Err(e) => json!({ "error": e }),
        };
        assert_eq!(norm(&split), norm(&c["split"]), "split {text:?}");
    }
    assert!(
        bad.is_empty(),
        "{} differences:\n{}",
        bad.len(),
        bad.iter().take(5).cloned().collect::<Vec<_>>().join("\n")
    );
}
