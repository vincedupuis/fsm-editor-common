//! The interface the editions call: one JSON request, `{"id": …, "fn": "<function>", "args": {…}}`, one JSON answer,
//! `{"id": …, "result": …}` or `{"id": …, "error": "<function>: <message>"}`. The same requests go through the
//! sidecar (`fsm-editor-common serve`: one request per line on stdin, one answer per line on stdout) and the
//! WebAssembly module (`common_alloc`, `common_call`, `common_free`). Every function takes texts and models and gives
//! texts and models: nothing reads or writes a file. Described by `api.schema.json`.

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::compare::{compare_models, describe_change, is_model_change, items_of};
use crate::convert::{ConvertInput, convert_group};
use crate::format::{
    apply_layout, assign_names, derived_ids, is_old_format, read_machine, read_model, resolve_references, write_machine,
};
use crate::grammar::check_text;
use crate::merge::{adopt, has_conflict_markers, split_conflicts};
use crate::model::{FsmModel, VertexType, machine_connection_points, normalize_model};
use crate::paths::decode_href;
use crate::validate::{SubmachineInfo, validate};
use crate::xmi::from_xmi;

#[derive(Deserialize)]
struct Request<'a> {
    #[serde(default, borrow)]
    id: Option<&'a RawValue>,
    #[serde(rename = "fn")]
    function: String,
    #[serde(default, borrow)]
    args: Option<&'a RawValue>,
}

type Args<'a> = Option<&'a RawValue>;

fn args<T: DeserializeOwned>(v: Args) -> Result<T, String> {
    serde_json::from_str(v.map_or("{}", |r| r.get())).map_err(|e| format!("bad arguments: {e}"))
}

fn out<T: Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string(v).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadResult<'a> {
    model: &'a FsmModel,
    lines: BTreeMap<&'a String, &'a usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    layout_problem: &'a Option<String>,
    placed: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteResult<'a> {
    model_text: &'a str,
    layout_text: &'a str,
    lines: BTreeMap<&'a String, &'a usize>,
    /// The model with what writing it gave (generated names, defaults, references resolved): only when it differs
    /// from the model sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a FsmModel>,
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

#[derive(Deserialize)]
struct ReadArgs {
    model: String,
    #[serde(default)]
    layout: Option<String>,
}

#[derive(Deserialize)]
struct ModelArg {
    model: FsmModel,
}

#[derive(Deserialize)]
struct ValidateArgs {
    model: FsmModel,
    #[serde(default)]
    submachines: HashMap<String, SubmachineInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckArgs {
    kind: String,
    text: String,
    #[serde(default)]
    allow_else: bool,
}

#[derive(Deserialize)]
struct CompareArgs {
    older: FsmModel,
    newer: FsmModel,
}

#[derive(Deserialize)]
struct AdoptArgs {
    result: FsmModel,
    source: FsmModel,
    keys: Vec<String>,
}

#[derive(Deserialize)]
struct ConvertArgs {
    inputs: Vec<ConvertInput>,
}

/// The submachine paths of a model, relative to its file (an old file's hrefs without their `#id`).
fn submachine_files(model: &FsmModel, old: bool) -> Vec<String> {
    model
        .vertices
        .iter()
        .filter(|v| v.vtype == VertexType::State)
        .filter_map(|v| v.submachine.as_ref().filter(|s| !s.is_empty()))
        .map(|s| if old { decode_href(s.split('#').next().unwrap_or("")) } else { s.clone() })
        .collect()
}

/// A model from a `.fsm` file's text, either format (an old one read as it is), and its layout file's text when given
/// (else every element is placed automatically). The error is the first problem, with its line.
fn load(text: &str, layout: Option<&str>) -> Result<(FsmModel, bool), String> {
    if is_old_format(text) {
        return from_xmi(text).map(|x| (x.model, true));
    }
    let mut read = read_model(text).map_err(|p| format!("Line {}: {}", p[0].line, p[0].message))?;
    apply_layout(&mut read.model, layout);
    Ok((read.model, false))
}

fn run(function: &str, a: Args) -> Result<String, String> {
    match function {
        "read" => {
            let a: ReadArgs = args(a)?;
            return match read_machine(&a.model, a.layout.as_deref()) {
                Ok((read, layout)) => out(&ReadResult {
                    model: &read.model,
                    lines: read.lines.iter().collect(),
                    layout_problem: &layout.problem,
                    placed: layout.placed,
                }),
                Err(problems) => out(&json!({ "problems": problems })),
            };
        }
        "write" => {
            let mut model = args::<ModelArg>(a)?.model;
            let sent = model.clone();
            normalize_model(&mut model);
            assign_names(&mut model, false);
            let w = write_machine(&model);
            if !w.lost.is_empty() {
                return out(&json!({ "lost": w.lost }));
            }
            resolve_references(&mut model, &w);
            let changed = model != sent;
            return out(&WriteResult {
                model_text: &w.model,
                layout_text: &w.layout,
                lines: w.lines.iter().collect(),
                model: if changed { Some(&model) } else { None },
            });
        }
        "validate" => {
            let a: ValidateArgs = args(a)?;
            return out(&validate(&a.model, &a.submachines));
        }
        "generate" => {
            let request = a.map_or("{}", |r| r.get());
            return String::from_utf8(fsm_cli::run_json(request.as_bytes())).map_err(|e| e.to_string());
        }
        _ => {}
    }
    let v: Value = match function {
        "version" => json!(env!("CARGO_PKG_VERSION")),
        "isOldFormat" => json!(is_old_format(&args::<Text>(a)?.text)),
        "hasConflictMarkers" => json!(has_conflict_markers(&args::<Text>(a)?.text)),
        // A machine's model from a file in either format (old files read as they are), for summaries, code
        // generation and traces: its name, its connection points and the files of its submachines.
        "load" => {
            let a: ReadArgs = args(a)?;
            match load(&a.model, a.layout.as_deref()) {
                Ok((model, old)) => json!({
                    "name": model.name,
                    "points": machine_connection_points(&model),
                    "defaultEntry": fsm_cli::model::has_default_entry(&crate::validate::to_cli_model(&model)),
                    "submachines": submachine_files(&model, old),
                    "old": old,
                    "model": model,
                }),
                Err(e) => json!({ "error": e }),
            }
        }
        // Of the model as written: generated names given first.
        "derivedIds" => {
            let mut model = args::<ModelArg>(a)?.model;
            normalize_model(&mut model);
            assign_names(&mut model, false);
            json!(derived_ids(&model, None).into_iter().collect::<BTreeMap<_, _>>())
        }
        "convert" => json!(convert_group(&args::<ConvertArgs>(a)?.inputs)),
        "checkText" => {
            let a: CheckArgs = args(a)?;
            match check_text(&a.kind, &a.text, a.allow_else) {
                Some(c) => json!(c),
                None => return Err(format!("unknown kind '{}'", a.kind)),
            }
        }
        // The changes from `older` to `newer`, each with its description (`described`: every property; `model`:
        // the model's only) and whether it shows when layout changes are hidden.
        "compare" => {
            let a: CompareArgs = args(a)?;
            let changes: Vec<Value> = compare_models(&a.older, &a.newer)
                .iter()
                .map(|c| {
                    let mut v = json!(c);
                    v["described"] = json!(describe_change(c, true));
                    v["modelDescribed"] = json!(describe_change(c, false));
                    v["modelChange"] = json!(is_model_change(c));
                    v
                })
                .collect();
            json!(changes)
        }
        // The id of each element by its key (see `compare`): what matches an element across versions.
        "keys" => {
            let model = args::<ModelArg>(a)?.model;
            let keys: serde_json::Map<String, Value> =
                items_of(&model).into_iter().map(|i| (i.key, Value::String(i.id))).collect();
            Value::Object(keys)
        }
        "splitConflicts" => match split_conflicts(&args::<Text>(a)?.text) {
            Ok(s) => json!(s),
            Err(e) => json!({ "error": e }),
        },
        "adopt" => {
            let a: AdoptArgs = args(a)?;
            json!(adopt(&a.result, &a.source, &a.keys))
        }
        _ => return Err(format!("unknown function '{function}'")),
    };
    out(&v)
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "internal error".into())
}

/// Answers one request (JSON text): never fails, never panics.
pub fn call(request: &[u8]) -> Vec<u8> {
    let error =
        |id: &str, e: String| format!("{{\"id\":{id},\"error\":{}}}", serde_json::to_string(&e).unwrap()).into_bytes();
    let Ok(text) = std::str::from_utf8(request) else { return error("null", "bad request: not UTF-8".into()) };
    match serde_json::from_str::<Request>(text) {
        Err(e) => error("null", format!("bad request: {e}")),
        Ok(r) => {
            let id = r.id.map_or("null", |i| i.get());
            match catch_unwind(AssertUnwindSafe(|| run(&r.function, r.args))) {
                Ok(Ok(result)) => format!("{{\"id\":{id},\"result\":{result}}}").into_bytes(),
                Ok(Err(e)) => error(id, format!("{}: {e}", r.function)),
                Err(p) => error(id, format!("{}: internal error: {}", r.function, panic_message(p.as_ref()))),
            }
        }
    }
}
