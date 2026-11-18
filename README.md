# fsm-editor-common

The file half of FSM Editor's three editions (VS Code, Visual Studio, JetBrains), written once in Rust: reading and writing a state machine's two files (`X.fsm`, the model, and `X.fsm.layout`, the drawing), converting the XMI files of earlier versions, validation, the text grammar's checks, comparing and merging two versions, and code generation. The `.fsm` format, its reader, validator, grammar and the code generator are [fsm-cli](https://github.com/vincedupuis/fsm-cli)'s, used as a library; the layout file's format is this repo's (`schemas/fsm-layout.schema.json`).

Nothing here reads or writes a file: every function takes texts and models, and gives texts and models.

## Two ways to run it

- **Sidecar** (Visual Studio, JetBrains): `fsm-editor-common serve` reads one JSON request per line on stdin and writes one JSON answer per line on stdout, in order. It holds no state and ends when stdin closes: a host starts it once, and starts it again if it exits.
- **WebAssembly** (VS Code, desktop and web): `fsm-editor-common.wasm`, without imports. It exports `memory`, `common_alloc(len) → ptr` (a buffer for the request), `common_call(ptr, len) → out` (answers the request and frees it; at `out`, the answer's length as a little-endian u32, then the answer) and `common_free(out, length + 4)`.

## Requests

```jsonc
→ {"id": 1, "fn": "read", "args": {"model": "{ \"format\": 1, … }", "layout": "{ … }"}}
← {"id": 1, "result": {"model": {…}, "lines": {"Idle": 7, …}, "placed": 0}}
→ {"id": 2, "fn": "checkText", "args": {"kind": "guard", "text": "a()&&b()"}}
← {"id": 2, "result": {"ok": true, "value": "a() && b()"}}
→ {"id": 3, "fn": "nope"}
← {"id": 3, "error": "nope: unknown function 'nope'"}
```

`schemas/api.schema.json` gives each function's arguments and result; `api-cases/cases.jsonl` holds requests and their answers, which each edition replays through its own client.

| Function | Arguments | Result |
| --- | --- | --- |
| `read` | `model`, `layout` texts | the editors' model (ids, absolute places), each element's line, the layout's problem; or the model file's problems |
| `load` | a file's text, either format | its model (an old file read as it is), name, connection points, submachine files |
| `write` | `model` | both files' texts and each element's line (the model back when writing changed it); or what the format can't hold |
| `derivedIds` | `model` | the ids fsm-cli gives its elements (in problems and generated code's traces), by the model's ids |
| `convert` | old files (`path`, `text`) | the new files, renames and losses, per file |
| `validate` | `model`, the submachines' summaries | problems by element id |
| `checkText` | `kind`, `text` | the normalized text, or the message |
| `compare` | `older`, `newer` | the changes, with their descriptions |
| `keys` | `model` | the id of each element by the key `compare` matches it by |
| `splitConflicts` | a text with Git's conflict markers | its two sides |
| `adopt` | `result`, `source`, `keys` | the result with that element as `source` has it |
| `generate` | fsm-cli's WebAssembly request | fsm-cli's response |
| `isOldFormat`, `hasConflictMarkers`, `version` | | |

## Building

fsm-cli is used by path: check it out beside this repo (`../fsm-cli`).

```sh
cargo build --release      # target/release/fsm-editor-common
cargo rustc --lib --crate-type cdylib --profile wasm --target wasm32-unknown-unknown
                           # target/wasm32-unknown-unknown/wasm/fsm_editor_common.wasm
cargo test
```

## License

MIT
