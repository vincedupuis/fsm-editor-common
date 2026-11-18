//! The layout file's JSON Schema (`schemas/fsm-layout.schema.json`) and the layout reader agree: both take the files
//! of `fixtures/layout/valid` and the examples' layout files, and both refuse those of `fixtures/layout/invalid`.

use std::fs;
use std::path::Path;

use fsm_cli::json;
use fsm_editor_common::format::layout_shape_problem;
use serde_json::Value;

fn files() -> Vec<(String, String, bool)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out = Vec::new();
    for (dir, valid) in [("layout/valid", true), ("layout/invalid", false), ("examples", true)] {
        let mut names: Vec<_> = fs::read_dir(root.join(dir))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|f| f.ends_with(".fsm.layout"))
            .collect();
        names.sort();
        assert!(!names.is_empty(), "{dir}");
        for f in names {
            out.push((format!("{dir}/{f}"), fs::read_to_string(root.join(dir).join(&f)).unwrap(), valid));
        }
    }
    out
}

#[test]
fn the_schema_agrees_with_the_files() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/fsm-layout.schema.json");
    let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let url = "file:///schemas/fsm-layout.schema.json";
    let mut schemas = boon::Schemas::new();
    let mut compiler = boon::Compiler::new();
    compiler.add_resource(url, value).unwrap();
    let index = compiler.compile(url, &mut schemas).unwrap();
    for (name, text, valid) in files() {
        let accepted = serde_json::from_str::<Value>(&text).is_ok_and(|v| schemas.validate(&v, index).is_ok());
        assert_eq!(accepted, valid, "{name}");
    }
}

#[test]
fn the_reader_agrees_with_the_files() {
    for (name, text, valid) in files() {
        let problem = match json::parse(&text) {
            Ok(root) => layout_shape_problem(&root),
            Err(e) => Some(e.message),
        };
        assert_eq!(problem.is_none(), valid, "{name}: {problem:?}");
    }
}
