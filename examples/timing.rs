//! Times the steps of `write` on a model JSON file: cargo run --release --example timing -- <model.json>

use std::time::Instant;

use fsm_editor_common::format::{assign_names, resolve_references, write_machine};
use fsm_editor_common::model::{FsmModel, normalize_model};

fn main() {
    let text = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let n = 50;
    let step = |label: &str, f: &mut dyn FnMut()| {
        let t = Instant::now();
        for _ in 0..n {
            f();
        }
        println!("{label:12} {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    };
    let mut model: FsmModel = serde_json::from_str(&text).unwrap();
    step("parse", &mut || model = serde_json::from_str(&text).unwrap());
    step("normalize", &mut || normalize_model(&mut model));
    step("names", &mut || {
        assign_names(&mut model, false);
    });
    let mut w = write_machine(&model);
    step("write", &mut || w = write_machine(&model));
    step("resolve", &mut || {
        resolve_references(&mut model, &w);
    });
    step("to json", &mut || {
        serde_json::to_string(&model).unwrap();
    });
}
