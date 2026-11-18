//! The interface's requests and answers (`api-cases/cases.jsonl`), which each edition replays through its own
//! client: every function, on the examples. `FSM_UPDATE_API_CASES=1 cargo test --test api` writes the file again
//! (check the change first).

use std::fs;
use std::path::Path;

use fsm_editor_common::api::call;
use serde_json::{Value, json};

fn example(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/examples").join(name)).unwrap()
}

fn old(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old").join(name)).unwrap()
}

fn ask(fun: &str, args: Value) -> Value {
    let req = json!({ "id": 1, "fn": fun, "args": args });
    serde_json::from_slice(&call(serde_json::to_vec(&req).unwrap().as_slice())).unwrap()
}

fn requests() -> Vec<Value> {
    let media = example("MediaPlayer.fsm");
    let media_layout = example("MediaPlayer.fsm.layout");
    let order = example("Order.fsm");
    let payment = example("Payment.fsm");
    let read = ask("read", json!({ "model": media, "layout": media_layout }))["result"]["model"].clone();
    let mut edited = read.clone();
    edited["vertices"][3]["entry"] = json!("changed()");
    edited["vertices"][3]["x"] = json!(edited["vertices"][3]["x"].as_f64().unwrap() + 30.0);
    let mut unnamed = read.clone();
    unnamed["vertices"].as_array_mut().unwrap().push(json!({
        "id": "new_state", "type": "state", "name": "", "parent": read["regions"][0]["id"], "x": 900, "y": 600, "w": 140, "h": 60, "regions": []
    }));
    let mut cannot = read.clone();
    cannot["transitions"].as_array_mut().unwrap().push(json!({
        "id": "bad", "source": "nowhere", "target": "Off", "kind": "external", "triggers": [], "guard": "", "action": ""
    }));
    let order_model = ask("read", json!({ "model": order }))["result"]["model"].clone();
    let pay = ask("load", json!({ "model": payment }))["result"].clone();
    let mut id = 0;
    let mut req = |fun: &str, args: Value| {
        id += 1;
        json!({ "id": id, "fn": fun, "args": args })
    };
    vec![
        req("version", json!({})),
        req("isOldFormat", json!({ "text": media })),
        req("isOldFormat", json!({ "text": "<?xml version=\"1.0\"?>\n<xmi:XMI/>" })),
        req("hasConflictMarkers", json!({ "text": "a\n<<<<<<< ours\nb\n=======\nc\n>>>>>>> theirs\n" })),
        req("read", json!({ "model": media, "layout": media_layout })),
        req("read", json!({ "model": order })),
        req("read", json!({ "model": media, "layout": "{ \"format\": 2 }" })),
        req("read", json!({ "model": "{ \"format\": 1, \"name\": 3 }" })),
        req("read", json!({ "model": "not json" })),
        req("load", json!({ "model": payment })),
        req("load", json!({ "model": order })),
        req("load", json!({ "model": "{" })),
        req("load", json!({ "model": old("MediaPlayer.fsm") })),
        req("write", json!({ "model": read })),
        req("write", json!({ "model": unnamed })),
        req("write", json!({ "model": cannot })),
        req("derivedIds", json!({ "model": unnamed })),
        req("convert", json!({ "inputs": [{ "path": "/x/MediaPlayer.fsm", "text": old("MediaPlayer.fsm") }] })),
        req("convert", json!({ "inputs": [{ "path": "/x/Broken.fsm", "text": "<xmi:XMI>" }] })),
        req(
            "validate",
            json!({ "model": order_model, "submachines": {
            "Payment.fsm": { "found": true, "name": pay["name"], "points": pay["points"] }
        } }),
        ),
        req("validate", json!({ "model": order_model })),
        req("checkText", json!({ "kind": "guard", "text": "a()&&!(b()||c())" })),
        req("checkText", json!({ "kind": "guard", "text": "else", "allowElse": true })),
        req("checkText", json!({ "kind": "guard", "text": "else" })),
        req("checkText", json!({ "kind": "actions", "text": "a();b();" })),
        req("checkText", json!({ "kind": "condition", "text": "x > 1" })),
        req("checkText", json!({ "kind": "trigger", "text": "after( 1.5 s )" })),
        req("checkText", json!({ "kind": "triggers", "text": "go, stop, after(2m)" })),
        req("checkText", json!({ "kind": "events", "text": "go, after(1s)" })),
        req("checkText", json!({ "kind": "event", "text": "eject" })),
        req("checkText", json!({ "kind": "name", "text": "Idle 2" })),
        req("checkText", json!({ "kind": "color", "text": "red" })),
        req("compare", json!({ "older": read, "newer": edited })),
        req("keys", json!({ "model": edited })),
        req("splitConflicts", json!({ "text": "a\n<<<<<<< ours\nb\n=======\nc\n>>>>>>> theirs\n" })),
        req("splitConflicts", json!({ "text": "<<<<<<<\nonly\n" })),
        req("adopt", json!({ "result": read, "source": edited, "keys": ["element@Stopped"] })),
        req(
            "generate",
            json!({
                "files": { "/Payment.fsm": payment },
                "generate": ["/Payment.fsm"],
                "template": "---\nlang: ts\n---\n{% filter file(machine.name ~ \".txt\") %}{{ machine.name }}: {{ states | length }} states{% endfilter %}",
                "templateName": "tiny.jinja"
            }),
        ),
        req("generate", json!({ "files": {}, "generate": ["/Nope.fsm"], "template": "x", "templateName": "x.hbs" })),
        req("nope", json!({})),
        req("read", json!({ "layout": 3 })),
    ]
}

#[test]
fn api_cases() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("api-cases/cases.jsonl");
    let mut lines = Vec::new();
    for r in requests() {
        let answer: Value = serde_json::from_slice(&call(serde_json::to_vec(&r).unwrap().as_slice())).unwrap();
        assert!(answer.get("result").is_some() || answer.get("error").is_some(), "{r}");
        lines.push(serde_json::to_string(&json!({ "request": r, "answer": answer })).unwrap());
    }
    let text = lines.join("\n") + "\n";
    if std::env::var("FSM_UPDATE_API_CASES").is_ok() {
        fs::write(&path, &text).unwrap();
    }
    let expected =
        fs::read_to_string(&path).expect("api-cases/cases.jsonl: FSM_UPDATE_API_CASES=1 cargo test --test api");
    for (got, want) in text.lines().zip(expected.lines()) {
        assert_eq!(got, want);
    }
    assert_eq!(text.lines().count(), expected.lines().count());
}

/// Every case's request and answer, and each function's arguments and result, as `schemas/api.schema.json` says.
#[test]
fn api_schema() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(root.join("schemas/api.schema.json")).unwrap()).unwrap();
    let url = "file:///schemas/api.schema.json";
    let mut schemas = boon::Schemas::new();
    let mut compiler = boon::Compiler::new();
    compiler.add_resource(url, schema.clone()).unwrap();
    let whole = compiler.compile(url, &mut schemas).unwrap();
    let text = fs::read_to_string(root.join("api-cases/cases.jsonl")).unwrap();
    let mut checked = 0;
    for line in text.lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let (req, ans) = (&case["request"], &case["answer"]);
        assert!(schemas.validate(req, whole).is_ok(), "request {req}");
        assert!(schemas.validate(ans, whole).is_ok(), "answer {ans}");
        let f = req["fn"].as_str().unwrap();
        if schema["$defs"]["functions"].get(f).is_none() || ans.get("error").is_some() {
            continue;
        }
        for (part, value) in [("args", &req["args"]), ("result", &ans["result"])] {
            let at = format!("{url}#/$defs/functions/{f}/{part}");
            let index = compiler.compile(&at, &mut schemas).unwrap();
            if let Err(e) = schemas.validate(value, index) {
                panic!("{f} {part}: {e}");
            }
        }
        checked += 1;
    }
    assert!(checked > 30);
}
