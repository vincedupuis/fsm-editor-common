//! Writing the files: a document of texts, numbers, lists and objects (fields in order), 2-space indented.

use std::collections::HashMap;
use std::fmt::Write as _;

use fsm_cli::json::{self, Json, Node};

/// A value to write.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Str(String),
    Num(f64),
    Bool(bool),
    Arr(Vec<Out>),
    Obj(Obj),
}

/// An object: its fields in order, and the ids of the element and transition it writes, whose lines the writer
/// reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Obj {
    pub fields: Vec<(String, Out)>,
    pub id: Option<String>,
    pub tid: Option<String>,
}

impl Obj {
    pub fn new() -> Obj {
        Obj::default()
    }

    /// Sets a field: in place when it is there, else at the end (a JavaScript object's assignment).
    pub fn set(&mut self, key: &str, value: Out) {
        match self.fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.fields.push((key.to_string(), value)),
        }
    }

    pub fn str(&mut self, key: &str, value: &str) {
        self.set(key, Out::Str(value.to_string()));
    }

    pub fn num(&mut self, key: &str, value: f64) {
        self.set(key, Out::Num(value));
    }

    pub fn get(&self, key: &str) -> Option<&Out> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// `Object.assign(this, other)`.
    pub fn assign(&mut self, other: &Obj) {
        for (k, v) in &other.fields {
            self.set(k, v.clone());
        }
    }
}

impl Out {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Out::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// The plain value of a node, as JavaScript holds it: an object's integer keys come first, in increasing order.
pub fn plain(n: &Node) -> Out {
    match &n.value {
        Json::Null => Out::Str(String::new()),
        Json::Bool(b) => Out::Bool(*b),
        Json::Number(x) => Out::Num(*x),
        Json::String(s) => Out::Str(s.clone()),
        Json::Array(items) => Out::Arr(items.iter().map(plain).collect()),
        Json::Object(fields) => {
            let mut o = Obj::new();
            for f in fields {
                o.set(&f.key, plain(&f.value));
            }
            js_key_order(&mut o);
            Out::Obj(o)
        }
    }
}

fn array_index(k: &str) -> Option<u32> {
    if k == "0" {
        return Some(0);
    }
    if k.is_empty() || k.starts_with('0') || !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    k.parse::<u32>().ok().filter(|&n| n < u32::MAX)
}

/// Orders an object's fields as a JavaScript object does: integer keys first, ascending, then the others in order.
pub fn js_key_order(o: &mut Obj) {
    if !o.fields.iter().any(|(k, _)| array_index(k).is_some()) {
        return;
    }
    let (mut ints, rest): (Vec<_>, Vec<_>) = o.fields.drain(..).partition(|(k, _)| array_index(k).is_some());
    ints.sort_by_key(|(k, _)| array_index(k).unwrap());
    o.fields = ints;
    o.fields.extend(rest);
}

/// A text as `JSON.stringify` writes it.
pub fn js_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `JSON.stringify(value)`: compact.
pub fn js_stringify(v: &Out) -> String {
    match v {
        Out::Str(s) => js_quote(s),
        Out::Num(n) => {
            if n.is_finite() {
                json::number(*n)
            } else {
                "null".into()
            }
        }
        Out::Bool(b) => b.to_string(),
        Out::Arr(items) => format!("[{}]", items.iter().map(js_stringify).collect::<Vec<_>>().join(",")),
        Out::Obj(o) => format!(
            "{{{}}}",
            o.fields.iter().map(|(k, x)| format!("{}:{}", js_quote(k), js_stringify(x))).collect::<Vec<_>>().join(",")
        ),
    }
}

/// Parses what `js_stringify` wrote back into a value (objects of texts, as the anchors that name nothing are).
pub fn js_parse(text: &str) -> Option<Out> {
    json::parse(text).ok().map(|n| plain(&n))
}

fn number(n: f64) -> String {
    if n.is_finite() {
        json::number(n)
    } else if n.is_nan() {
        "NaN".into()
    } else if n > 0.0 {
        "Infinity".into()
    } else {
        "-Infinity".into()
    }
}

fn flat(v: &Out) -> String {
    match v {
        Out::Str(s) => json::quote(s),
        Out::Num(n) => number(*n),
        Out::Bool(b) => b.to_string(),
        Out::Arr(items) => format!("[{}]", items.iter().map(flat).collect::<Vec<_>>().join(", ")),
        Out::Obj(o) => {
            if o.fields.is_empty() {
                "{}".into()
            } else {
                format!(
                    "{{ {} }}",
                    o.fields
                        .iter()
                        .map(|(k, x)| format!("{}: {}", json::quote(k), flat(x)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
    }
}

/// Lines of the objects that name an element or a transition: the first line each id is written on.
pub type Lines = HashMap<String, usize>;

struct Writer<'a> {
    lines: Vec<String>,
    inline: &'a dyn Fn(&Obj) -> bool,
    ids: Lines,
}

impl Writer<'_> {
    fn put(&mut self, v: &Out, indent: &str, prefix: &str, suffix: &str, top: bool) {
        match v {
            Out::Obj(o) => {
                let line = self.lines.len() + 1;
                for id in [&o.id, &o.tid].into_iter().flatten() {
                    self.ids.entry(id.clone()).or_insert(line);
                }
                if o.fields.is_empty() || (!top && (self.inline)(o)) {
                    self.lines.push(format!("{indent}{prefix}{}{suffix}", flat(v)));
                    return;
                }
                self.lines.push(format!("{indent}{prefix}{{"));
                let inner = format!("{indent}  ");
                let n = o.fields.len();
                for (i, (k, x)) in o.fields.iter().enumerate() {
                    self.put(x, &inner, &format!("{}: ", json::quote(k)), if i < n - 1 { "," } else { "" }, false);
                }
                self.lines.push(format!("{indent}}}{suffix}"));
            }
            Out::Arr(items) => {
                if items.is_empty() {
                    self.lines.push(format!("{indent}{prefix}[]{suffix}"));
                    return;
                }
                self.lines.push(format!("{indent}{prefix}["));
                let inner = format!("{indent}  ");
                let n = items.len();
                for (i, x) in items.iter().enumerate() {
                    self.put(x, &inner, "", if i < n - 1 { "," } else { "" }, false);
                }
                self.lines.push(format!("{indent}]{suffix}"));
            }
            _ => self.lines.push(format!("{indent}{prefix}{}{suffix}", flat(v))),
        }
    }
}

/// Writes a document, 2-space indented, with a newline at the end. `inline(o)` says whether an object goes on one
/// line (lists are always one item per line, the top-level object one field per line). Returns the text and the
/// line of each id the objects carry.
pub fn write_json(root: &Obj, inline: &dyn Fn(&Obj) -> bool) -> (String, Lines) {
    let mut w = Writer { lines: Vec::new(), inline, ids: HashMap::new() };
    w.put(&Out::Obj(root.clone()), "", "", "", true);
    let mut text = w.lines.join("\n");
    text.push('\n');
    (text, w.ids)
}
