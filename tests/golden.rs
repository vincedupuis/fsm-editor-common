//! The files written for many machines, against `tests/golden/`: each machine read and written again (the examples,
//! with their layout files and without, and every `.fsm` file of fsm-cli's cases: `FSM_CASES`, else
//! `../fsm-cli/tests/cases`), and the old (XMI) files converted, alone and together, with what converting reports
//! (the old examples, and `fixtures/old-edges`: broken structure, colors and groups edited by hand, region ids used
//! again, infinite coordinates, a file earlier versions changed on every save…). Each converted machine, read and
//! written again, must stay the same.
//!
//! The golden files are what the three editions wrote, byte for byte, when this code replaced theirs. Writing them
//! again (`FSM_UPDATE_GOLDEN=1 cargo test --test golden`) is a change of the files the editions write: check the diff.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use fsm_editor_common::convert::{ConvertInput, convert_group};
use fsm_editor_common::format::{assign_names, read_machine, write_machine};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cases() -> PathBuf {
    std::env::var("FSM_CASES").map(PathBuf::from).unwrap_or_else(|_| root().join("../fsm-cli/tests/cases"))
}

fn fsm_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fsm"))
        .collect();
    files.sort();
    files
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().to_string()
}

/// A machine read and written again: its files, or the problems of its model file.
fn write(out: &mut BTreeMap<String, String>, job: &str, model: &str, layout: Option<&str>) {
    match read_machine(model, layout) {
        Err(problems) => {
            let text: Vec<String> = problems.iter().map(|p| format!("line {}: {}", p.line, p.message)).collect();
            out.insert(format!("{job}/report.txt"), text.join("\n") + "\n");
        }
        Ok((mut read, l)) => {
            assign_names(&mut read.model, false);
            let w = write_machine(&read.model);
            out.insert(format!("{job}/model.fsm"), w.model);
            out.insert(format!("{job}/model.fsm.layout"), w.layout);
            let mut report =
                vec![format!("placed {}", l.placed), format!("layout problem {}", l.problem.as_deref().unwrap_or("-"))];
            report.extend(w.lost.iter().map(|x| format!("lost {x}")));
            out.insert(format!("{job}/report.txt"), report.join("\n") + "\n");
        }
    }
}

/// Old files converted together; each converted machine written again must stay the same.
fn convert(out: &mut BTreeMap<String, String>, job: &str, files: &[PathBuf]) {
    let inputs: Vec<ConvertInput> = files
        .iter()
        .map(|f| ConvertInput { path: format!("/x/{}", name(f)), text: fs::read_to_string(f).unwrap() })
        .collect();
    for r in convert_group(&inputs) {
        let file = r.path.rsplit('/').next().unwrap().to_string();
        if r.error.is_none() {
            out.insert(format!("{job}/{file}"), r.model_text.clone());
            out.insert(format!("{job}/{file}.layout"), r.layout_text.clone());
            let mut again = BTreeMap::new();
            write(&mut again, "x", &r.model_text, Some(&r.layout_text));
            assert_eq!(again.get("x/model.fsm"), Some(&r.model_text), "{job}/{file}: changes when written again");
            assert_eq!(
                again.get("x/model.fsm.layout"),
                Some(&r.layout_text),
                "{job}/{file}: its layout changes when written again"
            );
        }
        let mut report =
            vec![format!("error {}", r.error.as_deref().unwrap_or("-")), format!("generated {}", r.generated)];
        report.extend(r.renames.iter().map(|x| format!("rename {x}")));
        report.extend(r.losses.iter().map(|x| format!("loss {x}")));
        out.insert(format!("{job}/{file}.report.txt"), report.join("\n") + "\n");
    }
}

#[test]
fn golden_files() {
    let mut out = BTreeMap::new();
    let fixtures = root().join("tests/fixtures");
    for f in fsm_files(&fixtures.join("examples")) {
        let model = fs::read_to_string(&f).unwrap();
        let layout = fs::read_to_string(format!("{}.layout", f.display())).ok();
        write(&mut out, &format!("examples/{}", name(&f)), &model, layout.as_deref());
        write(&mut out, &format!("examples-placed/{}", name(&f)), &model, None);
    }
    let mut dirs: Vec<PathBuf> =
        fs::read_dir(cases()).expect("fsm-cli's cases").flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        for f in fsm_files(&d) {
            write(&mut out, &format!("cases/{}/{}", name(&d), name(&f)), &fs::read_to_string(&f).unwrap(), None);
        }
    }
    let old = fsm_files(&fixtures.join("old"));
    convert(&mut out, "old-examples/together", &old);
    for f in &old {
        convert(&mut out, &format!("old-examples/{}", name(f)), std::slice::from_ref(f));
    }
    for f in fsm_files(&fixtures.join("old-edges")) {
        convert(&mut out, &format!("old-edges/{}", name(&f)), std::slice::from_ref(&f));
    }

    let golden = root().join("tests/golden");
    if std::env::var("FSM_UPDATE_GOLDEN").is_ok() {
        let _ = fs::remove_dir_all(&golden);
        for (path, text) in &out {
            let p = golden.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }
    let mut differ = Vec::new();
    for (path, text) in &out {
        match fs::read_to_string(golden.join(path)) {
            Ok(g) if g == *text => {}
            Ok(_) => differ.push(format!("differs {path}")),
            Err(_) => differ.push(format!("missing {path}")),
        }
    }
    let mut expected = Vec::new();
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    walk(&golden, &golden, &mut expected);
    for p in expected {
        if !out.contains_key(&p) {
            differ.push(format!("no longer written {p}"));
        }
    }
    assert!(differ.is_empty(), "{} files differ from tests/golden:\n{}", differ.len(), differ.join("\n"));
    assert!(out.len() > 100);
}
