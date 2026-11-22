# fsm-editor-common

The file services of FSM Editor's three editions, in Rust (README.md): reading and writing a machine's two files, converting old XMI files, validation, the grammar's checks, comparing and merging, code generation. The VS Code edition runs it as WebAssembly, the Visual Studio and JetBrains editions as a sidecar process (`fsm-editor-common serve`, JSON lines). The editions keep their UI, geometry and editing; they send their model (the VS Code edition's `FsmModel` as JSON) and get texts and models back.

## Commands

```sh
cargo build --release                 # the sidecar, target/release/fsm-editor-common
cargo rustc --lib --crate-type cdylib --profile wasm --target wasm32-unknown-unknown
                                      # target/wasm32-unknown-unknown/wasm/fsm_editor_common.wasm
cargo test                            # fsm-cli's cases (FSM_CASES, else ../fsm-cli/tests/cases), the fixtures, the golden files, the interface
FSM_UPDATE_API_CASES=1 cargo test --test api   # writes api-cases/cases.jsonl: check the change first
FSM_UPDATE_GOLDEN=1 cargo test --test golden   # writes tests/golden: a change of the files the editions write, check the diff
cargo clippy --all-targets
cargo fmt                             # rustfmt.toml: 120 columns
```

fsm-cli is a path dependency (`../fsm-cli`): the umbrella repo's checkout, or in the workflows a checkout beside this one at `FSM_CLI_VERSION` (release.yml). After fsm-cli's version changes, `cargo update -p fsm-cli` so that `--locked` builds pass.

## Architecture

| Module | |
| --- | --- |
| `api.rs` | The requests: `call(json) → json`, one function per `fn`, errors as `<fn>: <message>`, panics caught. `schemas/api.schema.json` describes them; `tests/api.rs` writes and checks `api-cases/cases.jsonl`. |
| `main.rs` | `serve`: JSON lines on stdin and stdout. |
| `lib.rs` | The WebAssembly exports (`common_alloc`, `common_call`, `common_free`), wasm32 only. |
| `model.rs` | The editors' model (serde: camelCase, optional fields left out, `null` numbers read as 0), `normalize_model`, JavaScript's rounding and `Math.max`/`min`. |
| `format.rs` | The two files: reading (fsm-cli's JSON reader and shape check, the editors' builder with resolved anchors), the layout (shape, places, automatic places), writing (deterministic: write(read(x)) = x), generated names, derived ids, references resolved. |
| `out.rs` | The files' JSON writer (2-space indent, inline rules, the line of each element) and `JSON.stringify`. |
| `xml.rs`, `xmi.rs`, `convert.rs` | Old files: XML, XMI with UML DI, conversion of a group linked by submachines. |
| `validate.rs` | fsm-cli's validator on the editors' model. |
| `grammar.rs` | fsm-cli's grammar checks, with the text an editor keeps (normalized). |
| `compare.rs`, `merge.rs` | Matching two versions by name, the changes; conflict markers, taking one side's element. |
| `paths.rs` | `/`-separated paths, the hrefs of old files. |

## Rules

- **Byte for byte:** this is a port of the VS Code edition's `format.ts`, `json.ts`, `xmi.ts`, `xml.ts`, `convert.ts`, `compare.ts` and `merge.ts`; the files it writes are the editions' files. JavaScript's semantics are kept where they show in a file or a message: `Math.round` (`js_round`), `Math.hypot` (`js_hypot`), `String(n)` (`fsm_cli::json::number`), `JSON.stringify`, `Number(text)`, object key order, NaN in `Math.max`. `tests/golden` holds what the three editions wrote, byte for byte, when this code replaced theirs (the examples, fsm-cli's cases, old files with edge cases, converted alone and together): `tests/golden.rs` checks every change against it.
- **fsm-cli owns** the `.fsm` format, its reader's problems, the validator, the grammar and its messages, code generation. Don't fork them here: change fsm-cli, release it, then follow it here.
- **The layout file's format is this repo's:** change `schemas/fsm-layout.schema.json`, the reader and `tests/fixtures/layout` together.
- **The interface is a contract** with three editions: change `api.rs`, `schemas/api.schema.json` and `api-cases` together; adding a field to a result is compatible, removing or renaming one is not.
- **No file access, no global state:** every function takes its texts and models in the request.
- Out of scope, as in every edition: state machine inheritance/redefinition and the transition-oriented notation.

## Releasing

From the umbrella repo: `./release.sh common <version>`. The tag's workflow tests, then attaches the sidecar per platform, the `.wasm`, fsm-cli's templates, the schemas (model, layout, interface) and `api-cases`. The editions pin it with `FSM_COMMON_VERSION` in their `release.yml`.
