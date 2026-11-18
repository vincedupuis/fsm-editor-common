//! `fsm-editor-common serve`: the sidecar the Visual Studio and JetBrains editions run. One JSON request per line on
//! stdin, one JSON answer per line on stdout, in order (see api.rs). It holds no state, and ends when stdin closes.

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") if args.len() == 1 => {}
        Some("-v" | "--version") => {
            println!("fsm-editor-common {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {
            eprintln!(
                "Usage: fsm-editor-common serve\n\nAnswers JSON requests, one per line on stdin, one per line on stdout."
            );
            std::process::exit(2);
        }
    }
    // A panic is answered as an error (api::call); its message goes nowhere else.
    std::panic::set_hook(Box::new(|_| {}));
    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.split(b'\n') {
        let Ok(mut line) = line else { break };
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let mut answer = fsm_editor_common::api::call(&line);
        answer.push(b'\n');
        if stdout.write_all(&answer).and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
}
