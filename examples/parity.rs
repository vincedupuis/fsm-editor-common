//! The umbrella repo's parity check (`parity/check.mjs`) runs this on its manifest: each job's files written as the
//! editions write them, into `<out>/<job name>/`.
//!
//!     cargo run --release --example parity -- <manifest.tsv> <out dir>

use std::fs;
use std::path::Path;

use fsm_editor_common::convert::{ConvertInput, convert_group};
use fsm_editor_common::format::{assign_names, read_machine, write_machine};

fn save(dir: &Path, file: &str, text: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(file), text).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let manifest = fs::read_to_string(&args[1]).unwrap();
    let out = Path::new(&args[2]);
    for line in manifest.lines().filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split('\t').collect();
        let dir = out.join(parts[1]);
        if parts[0] == "write" {
            let model = fs::read_to_string(parts[2]).unwrap();
            let layout = if parts[3] == "-" { None } else { Some(fs::read_to_string(parts[3]).unwrap()) };
            match read_machine(&model, layout.as_deref()) {
                Err(problems) => {
                    let text: Vec<String> =
                        problems.iter().map(|p| format!("line {}: {}", p.line, p.message)).collect();
                    save(&dir, "report.txt", &(text.join("\n") + "\n"));
                }
                Ok((mut read, layout)) => {
                    assign_names(&mut read.model, false);
                    let w = write_machine(&read.model);
                    save(&dir, "model.fsm", &w.model);
                    save(&dir, "model.fsm.layout", &w.layout);
                    let mut report = vec![
                        format!("placed {}", layout.placed),
                        format!("layout problem {}", layout.problem.as_deref().unwrap_or("-")),
                    ];
                    report.extend(w.lost.iter().map(|l| format!("lost {l}")));
                    save(&dir, "report.txt", &(report.join("\n") + "\n"));
                }
            }
        } else {
            let inputs: Vec<ConvertInput> = parts[2..]
                .iter()
                .map(|spec| {
                    let (path, file) = spec.split_once('=').unwrap();
                    ConvertInput { path: path.to_string(), text: fs::read_to_string(file).unwrap() }
                })
                .collect();
            for r in convert_group(&inputs) {
                let file = r.path.rsplit('/').next().unwrap().to_string();
                if r.error.is_none() {
                    save(&dir, &file, &r.model_text);
                    save(&dir, &format!("{file}.layout"), &r.layout_text);
                }
                let mut report =
                    vec![format!("error {}", r.error.as_deref().unwrap_or("-")), format!("generated {}", r.generated)];
                report.extend(r.renames.iter().map(|l| format!("rename {l}")));
                report.extend(r.losses.iter().map(|l| format!("loss {l}")));
                save(&dir, &format!("{file}.report.txt"), &(report.join("\n") + "\n"));
            }
        }
    }
}
