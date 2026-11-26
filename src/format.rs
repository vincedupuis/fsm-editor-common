//! The file format: a machine is two JSON files, `<Name>.fsm` (the model, without ids: elements are identified by
//! their names) and `<Name>.fsm.layout` beside it (the drawing). The model file's format is fsm-cli's (its README,
//! "The .fsm file format", and its JSON Schemas); the layout file's is this crate's (`schemas/fsm-layout.schema.json`).
//!
//! Reading the model gives fsm-cli's problems (its JSON reader and shape check) and its ids, derived from the names
//! (an element: its name; a point: `<owner>.<point>`; `<owner>.initial`, `.history`, `.deepHistory`; the only region
//! of a state or of the machine: `<owner>.`; a transition: `<source>#<n>`; an id already taken gets `~2`, `~3`...).
//! The editions work on the flat model of model.rs, with absolute coordinates; the layout file holds them relative to
//! their owner (`apply_layout`, `write_machine`).
//!
//! A reference that names nothing is kept as it is: a transition's target is then UNRESOLVED followed by the
//! reference, a comment's anchor UNRESOLVED (a reference) or UNRESOLVED_ANCHOR (a transition, as JSON) followed by it.

use std::collections::{HashMap, HashSet};

use fsm_cli::format::{shape_problems, split_events};
use fsm_cli::json::{self, Field, Node};
use fsm_cli::model::{js_space, js_trim};
use serde::Serialize;

use crate::model::{
    FsmModel, ITEM_COLOR_KEYS, ItemColors, MachineKind, Point, PointKind, Region, RegionLayout, Transition,
    TransitionKind, Vertex, VertexType, is_color, js_max, js_min, n1, normalize_groups, s,
};
use crate::out::{Obj, Out, js_parse, js_stringify, plain, write_json};

pub const UNRESOLVED: char = '\u{e001}';
pub const UNRESOLVED_ANCHOR: char = '\u{e002}';
/// A comment's anchor to the machine itself, in the editors' model (the machine has no id of its own); the file names
/// it by the machine's name.
pub const MACHINE_ANCHOR: &str = "";

/// The reference of a target that names nothing ('' for an `initial` without `target`), or None.
pub fn unresolved(target: &str) -> Option<&str> {
    target.strip_prefix(UNRESOLVED)
}

/// A problem of a file's text or shape, with its line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

/// Whether a text is a file in the old format (XMI).
pub fn is_old_format(text: &str) -> bool {
    fsm_cli::format::is_old_format(text)
}

/// The size of each kind of vertex when the layout has none (the editors' defaults).
pub fn default_size(t: VertexType) -> (f64, f64) {
    match t {
        VertexType::State => (140.0, 60.0),
        VertexType::Final | VertexType::ShallowHistory | VertexType::DeepHistory => (26.0, 26.0),
        VertexType::Initial | VertexType::Terminate => (20.0, 20.0),
        VertexType::Choice => (28.0, 28.0),
        VertexType::Junction => (14.0, 14.0),
        VertexType::Fork | VertexType::Join => (90.0, 8.0),
        VertexType::EntryPoint | VertexType::ExitPoint | VertexType::ConnectionPointRef => (16.0, 16.0),
        VertexType::Comment => (160.0, 64.0),
    }
}

/// Kinds the editors let the user resize: the layout holds their width and height.
pub fn resizable(t: VertexType) -> bool {
    matches!(t, VertexType::State | VertexType::Comment | VertexType::Fork | VertexType::Join)
}

/// Item colors: their keys in the layout file, by model key.
fn item_color_file_key(k: &str) -> &'static str {
    match k {
        "background" => "background",
        "entry" => "entry",
        "exit" => "exit",
        "doActivity" => "do",
        "deferrable" => "defer",
        "invariant" => "invariant",
        "triggers" => "trigger",
        "guard" => "guard",
        "action" => "action",
        "precondition" => "precondition",
        _ => "postcondition",
    }
}

fn item_color_from_file(k: &str) -> Option<&'static str> {
    ITEM_COLOR_KEYS.iter().copied().find(|m| item_color_file_key(m) == k)
}

const ELEMENT_TYPES: [VertexType; 8] = [
    VertexType::State,
    VertexType::Choice,
    VertexType::Junction,
    VertexType::Fork,
    VertexType::Join,
    VertexType::Final,
    VertexType::Terminate,
    VertexType::Comment,
];
const REGION_FIELDS: [&str; 4] = ["initial", "history", "deepHistory", "elements"];
const PSEUDO_FIELDS: [(&str, VertexType); 3] = [
    ("initial", VertexType::Initial),
    ("history", VertexType::ShallowHistory),
    ("deepHistory", VertexType::DeepHistory),
];

fn is_element(t: VertexType) -> bool {
    ELEMENT_TYPES.contains(&t)
}

fn label(t: VertexType) -> &'static str {
    match t {
        VertexType::State => "state",
        VertexType::Final => "final state",
        VertexType::Initial => "initial pseudostate",
        VertexType::ShallowHistory => "shallow history",
        VertexType::DeepHistory => "deep history",
        VertexType::Choice => "choice",
        VertexType::Junction => "junction",
        VertexType::Fork => "fork",
        VertexType::Join => "join",
        VertexType::EntryPoint => "entry point",
        VertexType::ExitPoint => "exit point",
        VertexType::Terminate => "terminate pseudostate",
        VertexType::ConnectionPointRef => "connection point reference",
        VertexType::Comment => "comment",
    }
}

// ------------------------------------------------------------------ reading the model

fn text(node: &Node, key: &str) -> String {
    node.get(key).and_then(|f| f.value.str()).unwrap_or("").to_string()
}

fn list<'a>(node: &'a Node, key: &str) -> &'a [Node] {
    node.get(key).and_then(|f| f.value.items()).unwrap_or(&[])
}

fn get_str<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    node.get(key).and_then(|f| f.value.str())
}

fn get_num(node: &Node, key: &str) -> Option<f64> {
    node.get(key).and_then(|f| f.value.number())
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// A machine read from its model file.
#[derive(Debug, Clone)]
pub struct ReadMachine {
    pub model: FsmModel,
    /// Line of each element, region, point and transition, by id.
    pub lines: HashMap<String, usize>,
}

struct Builder<'a> {
    model: FsmModel,
    lines: HashMap<String, usize>,
    taken: HashSet<String>,
    by_name: HashMap<String, String>,
    pseudo: HashMap<String, String>,
    points: HashMap<String, String>,
    pending: Vec<(String, &'a Node, bool)>,
    /// Comments (by vertex index) and their anchors' nodes, resolved once every transition is known.
    anchors: Vec<(usize, &'a [Node])>,
}

impl<'a> Builder<'a> {
    fn alloc(&mut self, base: &str) -> String {
        let mut id = base.to_string();
        let mut n = 2;
        while self.taken.contains(&id) {
            id = format!("{base}~{n}");
            n += 1;
        }
        self.taken.insert(id.clone());
        id
    }

    fn named(&mut self, name: &str) -> String {
        let id = self.alloc(if name.is_empty() { "~" } else { name });
        self.by_name.entry(name.to_string()).or_insert_with(|| id.clone());
        id
    }

    fn region(&mut self, node: &'a Node, id: &str, owner: &str) {
        for (key, vtype) in PSEUDO_FIELDS {
            let Some(f) = node.get(key) else { continue };
            let pk = format!("{owner}.{key}");
            let vid = self.alloc(&pk);
            self.pseudo.entry(pk).or_insert_with(|| vid.clone());
            self.lines.insert(vid.clone(), f.line);
            self.model.vertices.push(Vertex::new(&vid, vtype, "", id));
            let x = &f.value;
            // The default transition, when there is one: a target or an action (an action without a target is
            // reported).
            if x.get("target").is_some() || x.get("action").is_some() {
                self.pending.push((vid, x, true));
            }
        }
        for e in list(node, "elements") {
            self.element(e, id);
        }
    }

    fn element(&mut self, e: &'a Node, parent: &str) {
        let name = text(e, "name");
        let id = self.named(&name);
        self.lines.insert(id.clone(), e.line);
        let vtype = VertexType::from_key(&text(e, "type")).filter(|t| is_element(*t)).unwrap_or(VertexType::Comment);
        let mut v = Vertex::new(&id, vtype, &name, parent);
        if vtype == VertexType::Comment {
            v.text = Some(text(e, "text"));
            v.anchors = Some(Vec::new());
            self.anchors.push((self.model.vertices.len(), list(e, "anchors")));
        }
        if vtype != VertexType::State {
            self.model.vertices.push(v);
            for t in list(e, "transitions") {
                self.pending.push((id.clone(), t, false));
            }
            return;
        }
        v.stereotype = non_empty(text(e, "stereotype"));
        v.entry = non_empty(text(e, "entry"));
        v.exit = non_empty(text(e, "exit"));
        v.do_activity = non_empty(text(e, "do"));
        v.invariant = non_empty(text(e, "invariant"));
        let defer = split_events(&text(e, "defer"));
        if !defer.is_empty() {
            v.deferrable = Some(defer);
        }
        if let Some(sub) = get_str(e, "submachine") {
            v.submachine = Some(sub.to_string());
        }
        let vi = self.model.vertices.len();
        self.model.vertices.push(v);
        for t in list(e, "transitions") {
            self.pending.push((id.clone(), t, false));
        }
        self.points_of(e, &id, &name, &id, true);
        let mut sources: Vec<(Region, &'a Node, String)> = Vec::new();
        if let Some(regs) = e.get("regions") {
            for r in regs.value.items().unwrap_or(&[]) {
                let rname = text(r, "name");
                let rid = self.named(&rname);
                self.lines.insert(rid.clone(), r.line);
                sources.push((Region { id: rid, name: rname.clone(), size: None }, r, rname));
            }
        } else if REGION_FIELDS.iter().any(|k| e.get(k).is_some()) {
            let rid = self.alloc(&format!("{name}."));
            self.lines.insert(rid.clone(), e.line);
            sources.push((Region { id: rid, name: String::new(), size: None }, e, name.clone()));
        }
        for (r, node, owner) in sources {
            let rid = r.id.clone();
            self.model.vertices[vi].regions.push(r);
            self.region(node, &rid, &owner);
        }
    }

    fn points_of(&mut self, node: &'a Node, owner_id: &str, owner_name: &str, parent: &str, state: bool) {
        let refs = state && node.get("submachine").is_some();
        for p in list(node, "points") {
            let name = text(p, "name");
            let entry = text(p, "type") == "entryPoint";
            let key = format!("{owner_name}.{name}");
            let id = self.alloc(&key);
            self.points.entry(key).or_insert_with(|| id.clone());
            self.lines.insert(id.clone(), p.line);
            let v = if refs {
                let mut v = Vertex::new(&id, VertexType::ConnectionPointRef, "", owner_id);
                v.point_ref = Some(name);
                v.point_kind = Some(if entry { PointKind::Entry } else { PointKind::Exit });
                v
            } else {
                Vertex::new(&id, if entry { VertexType::EntryPoint } else { VertexType::ExitPoint }, &name, parent)
            };
            self.model.vertices.push(v);
            for t in list(p, "transitions") {
                self.pending.push((id.clone(), t, false));
            }
        }
    }

    fn resolve(&self, reference: &str) -> String {
        let found = match reference.find('.') {
            None => self.by_name.get(reference),
            Some(dot) => {
                let rest = &reference[dot + 1..];
                if rest == "initial" || rest == "history" || rest == "deepHistory" {
                    self.pseudo.get(reference)
                } else {
                    self.points.get(reference)
                }
            }
        };
        found.cloned().unwrap_or_else(|| format!("{UNRESOLVED}{reference}"))
    }

    /// A comment's anchors, as ids: a reference names an element, an object a transition (under its source, by its
    /// trigger and target, then by order among identical ones). One that names nothing is kept: UNRESOLVED followed
    /// by the reference, UNRESOLVED_ANCHOR followed by the object as JSON.
    fn resolve_anchors(&mut self) {
        // A comment can be attached to an element, a region or the machine (by its name).
        let mut vertex_ids: HashSet<String> = self.model.vertices.iter().map(|v| v.id.clone()).collect();
        vertex_ids.extend(self.model.regions.iter().map(|r| r.id.clone()));
        vertex_ids.extend(self.model.vertices.iter().flat_map(|v| v.regions.iter().map(|r| r.id.clone())));
        let anchors = std::mem::take(&mut self.anchors);
        for (vi, nodes) in anchors {
            let mut used: HashSet<usize> = HashSet::new();
            let mut out = Vec::new();
            for a in nodes {
                if let Some(r) = a.str() {
                    if r == self.model.name && !r.is_empty() {
                        out.push(MACHINE_ANCHOR.to_string());
                        continue;
                    }
                    let id = self.resolve(r);
                    out.push(if vertex_ids.contains(&id) { id } else { format!("{UNRESOLVED}{r}") });
                    continue;
                }
                let source = self.resolve(&text(a, "source"));
                let trigger = split_events(&text(a, "trigger")).join(", ");
                let target = get_str(a, "target").map(|r| self.resolve(r));
                let found = self.model.transitions.iter().enumerate().position(|(i, x)| {
                    !used.contains(&i)
                        && x.source == source
                        && x.triggers.join(", ") == trigger
                        && match &target {
                            None => x.kind == TransitionKind::Internal,
                            Some(t) => x.kind != TransitionKind::Internal && x.target == *t,
                        }
                });
                match found {
                    Some(i) => {
                        used.insert(i);
                        out.push(self.model.transitions[i].id.clone());
                    }
                    None => out.push(format!("{UNRESOLVED_ANCHOR}{}", js_stringify(&plain(a)))),
                }
            }
            self.model.vertices[vi].anchors = Some(out);
        }
    }
}

/// Reads a model file. Returns its problems when it can't be read (not JSON, or a wrong shape).
pub fn read_model(source: &str) -> Result<ReadMachine, Vec<Problem>> {
    let root = json::parse(source).map_err(|e| vec![Problem { line: e.line, message: e.message }])?;
    let problems = shape_problems(&root);
    if !problems.is_empty() {
        return Err(problems.into_iter().map(|p| Problem { line: p.line, message: p.message }).collect());
    }
    let name = text(&root, "name");
    let mut model = FsmModel::new(&name);
    model.kind = if text(&root, "type") == "protocol" { MachineKind::Protocol } else { MachineKind::Behavioral };
    model.context = Some(text(&root, "context"));
    model.documentation = Some(text(&root, "documentation"));
    let mut b = Builder {
        model,
        lines: HashMap::new(),
        taken: HashSet::new(),
        by_name: HashMap::new(),
        pseudo: HashMap::new(),
        points: HashMap::new(),
        pending: Vec::new(),
        anchors: Vec::new(),
    };
    let mut sources: Vec<(&Node, String, String)> = Vec::new();
    if let Some(regs) = root.get("regions") {
        for r in regs.value.items().unwrap_or(&[]) {
            let rname = text(r, "name");
            let rid = b.named(&rname);
            b.lines.insert(rid.clone(), r.line);
            b.model.regions.push(Region { id: rid.clone(), name: rname.clone(), size: None });
            sources.push((r, rid, rname));
        }
    } else {
        let rid = b.alloc(&format!("{name}."));
        b.model.regions.push(Region { id: rid.clone(), name: String::new(), size: None });
        sources.push((&root, rid, name.clone()));
    }
    for (node, id, owner) in &sources {
        b.region(node, id, owner);
    }
    let top = b.model.regions[0].id.clone();
    b.points_of(&root, "", &name, &top, false);

    let mut count: HashMap<String, usize> = HashMap::new();
    let pending = std::mem::take(&mut b.pending);
    for (source, node, is_default) in pending {
        let n = count.entry(source.clone()).or_insert(0);
        *n += 1;
        let id = b.alloc(&format!("{source}#{n}"));
        b.lines.insert(id.clone(), node.line);
        let kind = match text(node, "type").as_str() {
            "internal" => TransitionKind::Internal,
            "local" => TransitionKind::Local,
            _ => TransitionKind::External,
        };
        let target = match get_str(node, "target") {
            Some(r) => b.resolve(r),
            None if kind == TransitionKind::Internal && !is_default => source.clone(),
            None => UNRESOLVED.to_string(),
        };
        let mut t = Transition::new(&id, &source, &target, kind);
        t.triggers = split_events(&text(node, "trigger"));
        t.guard = text(node, "guard");
        t.action = text(node, "action");
        t.precondition = non_empty(text(node, "precondition"));
        t.postcondition = non_empty(text(node, "postcondition"));
        b.model.transitions.push(t);
    }
    b.resolve_anchors();
    Ok(ReadMachine { model: b.model, lines: b.lines })
}

// ------------------------------------------------------------------ references

/// What the writer needs to know about the model: owners, names and references (of the model as it was when the
/// index was made).
pub(crate) struct Index {
    pub m: FsmModel,
    pub v: HashMap<String, usize>,
    pub t: HashMap<String, usize>,
    pub region_owner: HashMap<String, Option<usize>>,
    pub regions: HashMap<String, Region>,
    pub top: HashSet<String>,
}

/// A comment's anchor as the file writes it.
pub(crate) enum Anchor {
    /// The file can't hold it (an initial pseudostate, a transition from one).
    Undefined,
    /// It names nothing in the model.
    Null,
    Out(Out),
}

impl Index {
    pub fn new(model: &FsmModel) -> Index {
        let mut ix = Index {
            m: model.clone(),
            v: HashMap::new(),
            t: HashMap::new(),
            region_owner: HashMap::new(),
            regions: HashMap::new(),
            top: model.regions.iter().map(|r| r.id.clone()).collect(),
        };
        for r in &model.regions {
            ix.region_owner.insert(r.id.clone(), None);
            ix.regions.insert(r.id.clone(), r.clone());
        }
        for (i, v) in model.vertices.iter().enumerate() {
            ix.v.entry(v.id.clone()).or_insert(i);
            for r in &v.regions {
                ix.region_owner.insert(r.id.clone(), Some(i));
                ix.regions.insert(r.id.clone(), r.clone());
            }
        }
        for (i, t) in model.transitions.iter().enumerate() {
            ix.t.entry(t.id.clone()).or_insert(i);
        }
        ix
    }

    pub fn vertex(&self, id: &str) -> Option<&Vertex> {
        self.v.get(id).map(|&i| &self.m.vertices[i])
    }

    pub fn is_border(&self, v: &Vertex) -> bool {
        v.vtype == VertexType::ConnectionPointRef
            || (matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint) && self.v.contains_key(&v.parent))
    }

    pub fn is_machine_point(&self, v: &Vertex) -> bool {
        matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint) && self.top.contains(&v.parent)
    }

    /// The owner of a region's initial and history pseudostates in references: the region when it is named (an item
    /// of `regions`: always, when there are several), else its state, or the machine.
    fn region_ref_owner(&self, region_id: &str) -> String {
        let r = self.regions.get(region_id);
        let several = match self.region_owner.get(region_id) {
            Some(Some(o)) => self.m.vertices[*o].regions.len() > 1,
            _ => self.m.regions.len() > 1,
        };
        if several || r.is_some_and(|r| !r.name.is_empty()) {
            return r.map(|r| r.name.clone()).unwrap_or_default();
        }
        match self.region_owner.get(region_id) {
            Some(Some(o)) => self.m.vertices[*o].name.clone(),
            _ => self.m.name.clone(),
        }
    }

    /// The point name of a border vertex or machine point.
    pub fn point_name(v: &Vertex) -> String {
        if v.vtype == VertexType::ConnectionPointRef { s(&v.point_ref).to_string() } else { v.name.clone() }
    }

    /// The reference to a vertex.
    pub fn reference(&self, id: &str) -> Option<String> {
        if let Some(u) = unresolved(id) {
            return Some(u.to_string());
        }
        let Some(v) = self.vertex(id) else { return Some(id.to_string()) };
        match v.vtype {
            VertexType::Initial | VertexType::ShallowHistory | VertexType::DeepHistory => Some(format!(
                "{}.{}",
                self.region_ref_owner(&v.parent),
                match v.vtype {
                    VertexType::Initial => "initial",
                    VertexType::ShallowHistory => "history",
                    _ => "deepHistory",
                }
            )),
            _ if self.is_border(v) => Some(format!(
                "{}.{}",
                self.vertex(&v.parent).map(|p| p.name.as_str()).unwrap_or(""),
                Index::point_name(v)
            )),
            _ if self.is_machine_point(v) => Some(format!("{}.{}", self.m.name, v.name)),
            _ => Some(v.name.clone()),
        }
    }

    pub fn owner(&self, v: &Vertex) -> Option<usize> {
        if self.is_border(v) {
            self.v.get(&v.parent).copied()
        } else {
            self.region_owner.get(&v.parent).copied().flatten()
        }
    }

    fn chain(&self, id: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.v.get(id).copied();
        if let Some(i) = cur
            && self.m.vertices[i].vtype != VertexType::State
        {
            cur = self.owner(&self.m.vertices[i]);
        }
        let mut seen = HashSet::new();
        while let Some(i) = cur {
            if !seen.insert(i) {
                break;
            }
            out.push(i);
            cur = self.owner(&self.m.vertices[i]);
        }
        out
    }

    /// The innermost state that contains both ends of `t` (or is one of them), or None: its bend points are relative
    /// to it.
    pub fn frame(&self, t: &Transition) -> Option<usize> {
        let g: HashSet<usize> = self.chain(&t.target).into_iter().collect();
        self.chain(&t.source).into_iter().find(|v| g.contains(v))
    }

    /// A comment's anchor as the file writes it: a reference, or for a transition `{ source, trigger, target }`; one
    /// that named nothing as it was read.
    pub fn anchor(&self, a: &str) -> Anchor {
        if let Some(r) = a.strip_prefix(UNRESOLVED) {
            return Anchor::Out(Out::Str(r.to_string()));
        }
        if let Some(r) = a.strip_prefix(UNRESOLVED_ANCHOR) {
            return match js_parse(r) {
                Some(o) => Anchor::Out(o),
                None => Anchor::Null,
            };
        }
        if a == MACHINE_ANCHOR {
            return Anchor::Out(Out::Str(self.m.name.clone()));
        }
        if self.v.contains_key(a) {
            return match self.reference(a) {
                Some(r) => Anchor::Out(Out::Str(r)),
                None => Anchor::Undefined,
            };
        }
        // A region, by its name (one anchored is given a name when it has none: `assign_names`).
        if let Some(r) = self.regions.get(a) {
            return if r.name.is_empty() { Anchor::Null } else { Anchor::Out(Out::Str(r.name.clone())) };
        }
        let Some(&ti) = self.t.get(a) else { return Anchor::Null };
        let t = &self.m.transitions[ti];
        let source = self.reference(&t.source);
        let (Some(source), true) = (source, self.v.contains_key(&t.source)) else { return Anchor::Undefined };
        let mut ao = Obj::new();
        ao.str("source", &source);
        if !t.triggers.is_empty() {
            ao.str("trigger", &t.triggers.join(", "));
        }
        if t.kind != TransitionKind::Internal
            && let Some(target) = self.reference(&t.target)
        {
            ao.str("target", &target);
        }
        Anchor::Out(Out::Obj(ao))
    }
}

/// How `model` refers to its element or transition `id` in a comment's anchor, kept as a reference that names nothing
/// (UNRESOLVED or UNRESOLVED_ANCHOR followed by it): for a model where it doesn't exist (yet). None when it can't be
/// referred to.
pub fn unresolved_anchor(model: &FsmModel, id: &str) -> Option<String> {
    match Index::new(model).anchor(id) {
        Anchor::Out(Out::Str(s)) => Some(format!("{UNRESOLVED}{s}")),
        Anchor::Out(o) => Some(format!("{UNRESOLVED_ANCHOR}{}", js_stringify(&o))),
        _ => None,
    }
}

/// How `model` refers to its vertex `id` (a transition's target), or None when it can't be referred to.
pub fn reference_of(model: &FsmModel, id: &str) -> Option<String> {
    Index::new(model).reference(id)
}

// ------------------------------------------------------------------ writing

/// What writing a machine gives.
#[derive(Debug, Clone, Default)]
pub struct Written {
    /// The model file.
    pub model: String,
    /// The layout file.
    pub layout: String,
    /// Line of each vertex and transition in the model file, by id.
    pub lines: HashMap<String, usize>,
    /// What the format can't hold, which was left out (an editor's model holds none of it).
    pub lost: Vec<String>,
}

fn finite(n: f64) -> bool {
    n1(n).is_finite()
}

fn xy(x: f64, y: f64) -> Obj {
    let mut o = Obj::new();
    o.num("x", n1(x));
    o.num("y", n1(y));
    o
}

fn colors(color: &Option<String>, items: &Option<ItemColors>, into: &mut Obj) {
    if let Some(c) = color
        && is_color(c)
    {
        into.str("color", c);
    }
    let mut ic = Obj::new();
    if let Some(items) = items {
        for k in ITEM_COLOR_KEYS {
            if let Some(c) = items.get(k)
                && is_color(c)
            {
                ic.str(item_color_file_key(k), c);
            }
        }
    }
    if !ic.is_empty() {
        into.set("itemColors", Out::Obj(ic));
    }
}

fn truthy(o: &Option<String>) -> bool {
    o.as_deref().is_some_and(|s| !s.is_empty())
}

struct Writer<'a> {
    model: &'a FsmModel,
    ix: Index,
    lost: Vec<String>,
    done: HashSet<usize>,
    done_t: HashSet<usize>,
    outgoing: HashMap<String, Vec<usize>>,
}

impl Writer<'_> {
    fn describe(&self, v: &Vertex) -> String {
        if v.name.is_empty() {
            format!("an unnamed {}", label(v.vtype))
        } else {
            format!("{} '{}'", label(v.vtype), v.name)
        }
    }

    fn describe_t(&self, t: &Transition) -> String {
        let from = match self.ix.vertex(&t.source) {
            Some(s) => self.describe(s),
            None => format!("'{}'", t.source),
        };
        let on = if t.triggers.is_empty() { String::new() } else { format!(" on {}", t.triggers.join(", ")) };
        format!("the transition from {from}{on}")
    }

    fn outs(&self, id: &str) -> Vec<usize> {
        self.outgoing.get(id).cloned().unwrap_or_default()
    }

    /// A vertex's place relative to `at` (a place that isn't a number, from an old file, is the origin's).
    fn position(&mut self, v: &Vertex, at: (f64, f64)) -> Obj {
        let dx = v.x - at.0;
        let dy = v.y - at.1;
        if finite(dx) && finite(dy) {
            return xy(dx, dy);
        }
        self.lost.push(format!("the place of {}, which is not a number", self.describe(v)));
        xy(if finite(dx) { dx } else { 0.0 }, if finite(dy) { dy } else { 0.0 })
    }

    /// Where the places inside `v` are measured from.
    fn origin(&self, v: Option<usize>) -> (f64, f64) {
        match v {
            Some(i) => {
                let v = &self.model.vertices[i];
                (if finite(v.x) { v.x } else { 0.0 }, if finite(v.y) { v.y } else { 0.0 })
            }
            None => (0.0, 0.0),
        }
    }

    /// A transition's fields in the model file, its layout entry (None when it has nothing to store) and that entry
    /// without what it stores; None when the file can't hold it (a transition to an initial pseudostate).
    fn transition(&mut self, ti: usize) -> Option<(Obj, Option<Obj>, Obj)> {
        self.done_t.insert(ti);
        let t = &self.model.transitions[ti];
        let mut o = Obj::new();
        o.str("type", t.kind.key());
        if !t.triggers.is_empty() {
            o.str("trigger", &t.triggers.join(", "));
        }
        if !t.guard.is_empty() {
            o.str("guard", &t.guard);
        }
        if !t.action.is_empty() {
            o.str("action", &t.action);
        }
        if truthy(&t.precondition) {
            o.str("precondition", s(&t.precondition));
        }
        if truthy(&t.postcondition) {
            o.str("postcondition", s(&t.postcondition));
        }
        let mut target = None;
        if t.kind == TransitionKind::Internal {
            if t.target != t.source {
                let d = self.describe_t(t);
                self.lost.push(format!("the target of the internal {}", &d[4..]));
            }
        } else {
            match self.ix.reference(&t.target) {
                None => {
                    let d = self.describe_t(t);
                    self.lost.push(format!("{d} to an initial pseudostate"));
                    return None;
                }
                Some(r) => {
                    o.str("target", &r);
                    target = Some(r);
                }
            }
        }
        o.tid = Some(t.id.clone());
        let trigger = o.get("trigger").and_then(Out::as_str).map(str::to_string);
        let mut key = Obj::new();
        if let Some(tr) = &trigger {
            key.str("trigger", tr);
        }
        if let Some(r) = &target {
            key.str("target", r);
        }
        let layout = self.transition_layout(ti, trigger, target);
        Some((o, layout, key))
    }

    fn transition_layout(&mut self, ti: usize, trigger: Option<String>, target: Option<String>) -> Option<Obj> {
        let t = &self.model.transitions[ti];
        let mut l = Obj::new();
        if let Some(tr) = trigger.filter(|s| !s.is_empty()) {
            l.str("trigger", &tr);
        }
        if let Some(r) = target {
            l.str("target", &r);
        }
        let keys = l.len();
        if let Some(points) = t.points.as_ref().filter(|p| !p.is_empty())
            && t.kind != TransitionKind::Internal
        {
            let f = self.origin(self.ix.frame(t));
            let bends: Vec<&Point> = points.iter().filter(|p| finite(p.x - f.0) && finite(p.y - f.1)).collect();
            if bends.len() < points.len() {
                let d = self.describe_t(t);
                self.lost.push(format!("bend points of {d} that are not numbers"));
            }
            if !bends.is_empty() {
                l.set("bends", Out::Arr(bends.iter().map(|p| Out::Obj(xy(p.x - f.0, p.y - f.1))).collect()));
            }
        }
        if let Some(lo) = t.label_offset
            && t.kind != TransitionKind::Internal
        {
            if finite(lo.x) && finite(lo.y) {
                l.set("label", Out::Obj(xy(lo.x, lo.y)));
            } else {
                let d = self.describe_t(t);
                self.lost.push(format!("the label position of {d}, which is not a number"));
            }
        }
        colors(&t.color, &t.item_colors, &mut l);
        if l.len() > keys { Some(l) } else { None }
    }

    /// The transitions leaving `v`, into the model object and layout entry. Layout entries are found by trigger and
    /// target, then by order among identical ones: a transition with nothing to store gets an entry anyway (its
    /// trigger and target) when a later one with the same trigger and target has one, so that each keeps its own.
    fn transitions_of(&mut self, vi: usize, o: &mut Obj, l: &mut Obj) {
        let list = self.outs(&self.model.vertices[vi].id);
        if list.is_empty() {
            return;
        }
        let mut mo = Vec::new();
        let mut written: Vec<(Option<Obj>, Obj, String)> = Vec::new();
        for ti in list {
            let Some((to, entry, key)) = self.transition(ti) else { continue };
            mo.push(Out::Obj(to));
            let k = js_stringify(&Out::Obj(key.clone()));
            written.push((entry, key, k));
        }
        let keep: Vec<bool> = (0..written.len())
            .map(|i| written[i + 1..].iter().any(|(e, _, k2)| e.is_some() && *k2 == written[i].2))
            .collect();
        let mut lo = Vec::new();
        for ((entry, key, _), keep) in written.into_iter().zip(keep) {
            if let Some(e) = entry {
                lo.push(Out::Obj(e));
            } else if keep {
                lo.push(Out::Obj(key));
            }
        }
        if !mo.is_empty() {
            o.set("transitions", Out::Arr(mo));
        }
        if !lo.is_empty() {
            l.set("transitions", Out::Arr(lo));
        }
    }

    fn points_of(&mut self, owner: Option<usize>, points: Vec<usize>, o: &mut Obj, l: &mut Obj) {
        if points.is_empty() {
            return;
        }
        let mut mo = Vec::new();
        let mut lo = Vec::new();
        let at = self.origin(owner);
        for pi in points {
            self.done.insert(pi);
            let p = &self.model.vertices[pi];
            // On a submachine state, the points are references to the submachine's; on another state, its own.
            if let Some(oi) = owner {
                let ov = &self.model.vertices[oi];
                let sub = truthy(&ov.submachine);
                if (p.vtype == VertexType::ConnectionPointRef) != sub {
                    let msg = format!(
                        "{} on {}, which {} a submachine state",
                        self.describe(p),
                        self.describe(ov),
                        if sub { "is" } else { "is not" }
                    );
                    self.lost.push(msg);
                    for t in self.outs(&p.id) {
                        self.done_t.insert(t);
                    }
                    continue;
                }
            }
            let kind = if p.vtype == VertexType::ConnectionPointRef {
                if p.point_kind == Some(PointKind::Exit) { "exitPoint" } else { "entryPoint" }
            } else {
                p.vtype.key()
            };
            let mut po = Obj::new();
            po.str("name", &Index::point_name(p));
            po.str("type", kind);
            let mut pl = Obj::new();
            pl.str("name", &Index::point_name(p));
            let pos = self.position(p, at);
            pl.assign(&pos);
            colors(&p.color, &p.item_colors, &mut pl);
            if truthy(&p.group) && owner.is_none() {
                pl.str("group", s(&p.group));
            }
            self.transitions_of(pi, &mut po, &mut pl);
            po.id = Some(p.id.clone());
            mo.push(Out::Obj(po));
            lo.push(Out::Obj(pl));
        }
        if !mo.is_empty() {
            o.set("points", Out::Arr(mo));
        }
        if !lo.is_empty() {
            l.set("points", Out::Arr(lo));
        }
    }

    /// The fields of region `r` (owned by `owner`, None at the top level) into `o` and `l`.
    fn region_fields(&mut self, r: &Region, owner: Option<usize>, o: &mut Obj, l: &mut Obj) {
        // Each element is written once (a region id used twice in a file edited by hand could hold a state's own
        // owner).
        let all: Vec<usize> = (0..self.model.vertices.len())
            .filter(|&i| {
                let v = &self.model.vertices[i];
                v.parent == r.id && !self.ix.is_machine_point(v) && !self.ix.is_border(v)
            })
            .collect();
        let kids: Vec<usize> = all.iter().copied().filter(|i| !self.done.contains(i)).collect();
        if kids.len() < all.len() {
            let whose = match owner {
                Some(oi) => self.describe(&self.model.vertices[oi]),
                None => "the state machine".to_string(),
            };
            self.lost.push(format!(
                "the region of {whose} as it was: its id is used twice, and its elements are written once"
            ));
        }
        let at = self.origin(owner);
        for (key, vtype) in PSEUDO_FIELDS {
            let found: Vec<usize> = kids.iter().copied().filter(|&i| self.model.vertices[i].vtype == vtype).collect();
            if found.is_empty() {
                continue;
            }
            let vi = found[0];
            for &x in &found[1..] {
                self.done.insert(x);
                let d = self.describe(&self.model.vertices[x]);
                self.lost.push(format!("{d}: a region has at most one"));
            }
            self.done.insert(vi);
            let v = &self.model.vertices[vi];
            let mut po = Obj::new();
            let mut pl = self.position(v, at);
            if truthy(&v.group) {
                pl.str("group", s(&v.group));
            }
            let outs = self.outs(&v.id);
            if let Some(&ti) = outs.first() {
                self.done_t.insert(ti);
                let t = &self.model.transitions[ti];
                match self.ix.reference(&t.target) {
                    None => self.lost.push(format!("the {key} transition to an initial pseudostate")),
                    Some(target) => {
                        if !target.is_empty() || unresolved(&t.target).is_none() {
                            po.str("target", &target);
                        }
                    }
                }
                if !t.action.is_empty() {
                    po.str("action", &t.action);
                }
                if !t.guard.is_empty()
                    || !t.triggers.is_empty()
                    || t.kind != TransitionKind::External
                    || truthy(&t.precondition)
                    || truthy(&t.postcondition)
                {
                    self.lost.push(format!("the guard, triggers, kind or conditions of the {key} transition"));
                }
                // Its route, when the file holds the transition (a target or an action).
                let tl = self.transition_layout(ti, None, None);
                if let Some(tl) = tl
                    && (po.has("target") || po.has("action"))
                {
                    pl.assign(&tl);
                }
                po.tid = Some(self.model.transitions[ti].id.clone());
            }
            for &x in outs.iter().skip(1) {
                self.done_t.insert(x);
                self.lost.push(format!("a second transition leaving the {key} pseudostate"));
            }
            po.id = Some(self.model.vertices[vi].id.clone());
            o.set(key, Out::Obj(po));
            l.set(key, Out::Obj(pl));
        }
        let mut mo = Vec::new();
        let mut lo = Vec::new();
        for vi in kids {
            let vtype = self.model.vertices[vi].vtype;
            if matches!(vtype, VertexType::Initial | VertexType::ShallowHistory | VertexType::DeepHistory)
                || self.done.contains(&vi)
            {
                continue;
            }
            if !is_element(vtype) {
                let d = self.describe(&self.model.vertices[vi]);
                self.lost.push(format!("{d}, which only goes on the border of a state or at the top level"));
                continue;
            }
            let (eo, el) = self.element(vi, at);
            mo.push(Out::Obj(eo));
            lo.push(Out::Obj(el));
        }
        // A composite state's only region, empty: written so that the state stays composite.
        let only = owner.is_some_and(|oi| self.model.vertices[oi].regions.len() == 1);
        if !mo.is_empty() || (only && !o.has("initial") && !o.has("history") && !o.has("deepHistory")) {
            o.set("elements", Out::Arr(mo));
        }
        if !lo.is_empty() {
            l.set("elements", Out::Arr(lo));
        }
    }

    fn regions_of(&mut self, owner: Option<usize>, regions: &[Region], o: &mut Obj, l: &mut Obj) {
        // A single region without a name is written as its owner's fields; a named one in `regions`.
        if regions.len() == 1 && regions[0].name.is_empty() {
            self.region_fields(&regions[0], owner, o, l);
            return;
        }
        let mut mo = Vec::new();
        let mut lo = Vec::new();
        for r in regions {
            let mut ro = Obj::new();
            ro.str("name", &r.name);
            let mut rl = Obj::new();
            rl.str("name", &r.name);
            // A region's size is its extent beside the others: a single region has none.
            if owner.is_some()
                && regions.len() > 1
                && let Some(size) = r.size
                && finite(size)
                && n1(size) > 0.0
            {
                rl.num("size", n1(size));
            }
            self.region_fields(r, owner, &mut ro, &mut rl);
            ro.id = Some(r.id.clone());
            mo.push(Out::Obj(ro));
            lo.push(Out::Obj(rl));
        }
        o.set("regions", Out::Arr(mo));
        l.set("regions", Out::Arr(lo));
    }

    fn element(&mut self, vi: usize, at: (f64, f64)) -> (Obj, Obj) {
        self.done.insert(vi);
        let model = self.model;
        let v = &model.vertices[vi];
        let mut o = Obj::new();
        o.str("name", &v.name);
        o.str("type", v.vtype.key());
        let mut l = Obj::new();
        l.str("name", &v.name);
        let pos = self.position(v, at);
        l.assign(&pos);
        if resizable(v.vtype) {
            let (dw, dh) = default_size(v.vtype);
            l.num("width", n1(if finite(v.w) && v.w > 0.0 { v.w } else { dw }));
            l.num("height", n1(if finite(v.h) && v.h > 0.0 { v.h } else { dh }));
        }
        colors(&v.color, &v.item_colors, &mut l);
        if truthy(&v.group) {
            l.str("group", s(&v.group));
        }
        o.id = Some(v.id.clone());
        match v.vtype {
            VertexType::State => {
                for (key, value) in [
                    ("stereotype", &v.stereotype),
                    ("submachine", &v.submachine),
                    ("entry", &v.entry),
                    ("do", &v.do_activity),
                    ("exit", &v.exit),
                ] {
                    if truthy(value) {
                        o.str(key, s(value));
                    }
                }
                if let Some(d) = v.deferrable.as_ref().filter(|d| !d.is_empty()) {
                    o.str("defer", &d.join(", "));
                }
                if truthy(&v.invariant) {
                    o.str("invariant", s(&v.invariant));
                }
                if v.regions.len() > 1
                    && let Some(layout) = v.region_layout
                {
                    l.str("regionLayout", if layout == RegionLayout::Horizontal { "horizontal" } else { "vertical" });
                }
                self.transitions_of(vi, &mut o, &mut l);
                let points: Vec<usize> = (0..model.vertices.len())
                    .filter(|&i| {
                        let p = &model.vertices[i];
                        p.parent == v.id
                            && matches!(
                                p.vtype,
                                VertexType::EntryPoint | VertexType::ExitPoint | VertexType::ConnectionPointRef
                            )
                    })
                    .collect();
                self.points_of(Some(vi), points, &mut o, &mut l);
                if !v.regions.is_empty() {
                    self.regions_of(Some(vi), &v.regions, &mut o, &mut l);
                }
            }
            VertexType::Comment => {
                if truthy(&v.text) {
                    o.str("text", s(&v.text));
                }
                let mut anchors = Vec::new();
                for a in v.anchors.iter().flatten() {
                    match self.ix.anchor(a) {
                        Anchor::Null => {
                            let d = self.describe(v);
                            self.lost
                                .push(format!("the anchor of {d} to '{a}', which is not an element or a transition"));
                        }
                        Anchor::Out(w) => anchors.push(w),
                        Anchor::Undefined => {
                            let d = self.describe(v);
                            if self.ix.v.contains_key(a.as_str()) {
                                self.lost.push(format!("the anchor of {d} to an initial pseudostate"));
                            } else {
                                let t = &self.ix.m.transitions[self.ix.t[a.as_str()]];
                                let dt = self.describe_t(t);
                                self.lost.push(format!("the anchor of {d} to {dt}"));
                            }
                        }
                    }
                }
                if !anchors.is_empty() {
                    o.set("anchors", Out::Arr(anchors));
                }
            }
            VertexType::Final | VertexType::Terminate => {
                if !v.regions.is_empty() || truthy(&v.entry) || truthy(&v.exit) || truthy(&v.do_activity) {
                    let d = self.describe(v);
                    self.lost.push(format!("the regions and behaviors of {d}"));
                }
                let outs = self.outs(&v.id);
                if !outs.is_empty() {
                    for t in outs {
                        self.done_t.insert(t);
                    }
                    let d = self.describe(v);
                    self.lost.push(format!("the transitions leaving {d}"));
                }
            }
            _ => self.transitions_of(vi, &mut o, &mut l),
        }
        (o, l)
    }
}

/// Writes a machine: its model and layout files. Names must have been given (`assign_names`).
pub fn write_machine(model: &FsmModel) -> Written {
    let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, t) in model.transitions.iter().enumerate() {
        outgoing.entry(t.source.clone()).or_default().push(i);
    }
    let mut w = Writer {
        model,
        ix: Index::new(model),
        lost: Vec::new(),
        done: HashSet::new(),
        done_t: HashSet::new(),
        outgoing,
    };
    let mut mo = Obj::new();
    mo.num("version", 1.0);
    mo.str("name", &model.name);
    mo.str("type", if model.kind == MachineKind::Protocol { "protocol" } else { "behavioral" });
    let mut lo = Obj::new();
    lo.num("version", 1.0);
    if truthy(&model.context) {
        mo.str("context", s(&model.context));
    }
    if truthy(&model.documentation) {
        mo.str("documentation", s(&model.documentation));
    }
    let machine_points: Vec<usize> =
        (0..model.vertices.len()).filter(|&i| w.ix.is_machine_point(&model.vertices[i])).collect();
    w.points_of(None, machine_points, &mut mo, &mut lo);
    w.regions_of(None, &model.regions, &mut mo, &mut lo);
    for (i, v) in model.vertices.iter().enumerate() {
        if !w.done.contains(&i) {
            let d = w.describe(v);
            w.lost.push(format!("{d}, which is not inside a known region"));
        }
    }
    for (i, t) in model.transitions.iter().enumerate() {
        if w.done_t.contains(&i) {
            continue;
        }
        let msg = match w.ix.vertex(&t.source) {
            Some(s) if s.vtype == VertexType::Comment => format!("a transition leaving {}", w.describe(s)),
            _ => format!("{}, whose source does not exist", w.describe_t(t)),
        };
        w.lost.push(msg);
    }
    let (model_text, lines) =
        write_json(&mo, &|o: &Obj| o.fields.iter().all(|(_, x)| !matches!(x, Out::Arr(_) | Out::Obj(_))));
    let (layout_text, _) =
        write_json(&lo, &|o: &Obj| o.fields.len() == 2 && o.fields[0].0 == "x" && o.fields[1].0 == "y");
    Written { model: model_text, layout: layout_text, lines, lost: w.lost }
}

// ------------------------------------------------------------------ names

fn generated_prefix(t: VertexType) -> &'static str {
    match t {
        VertexType::State => "state",
        VertexType::Choice => "choice",
        VertexType::Junction => "junction",
        VertexType::Fork => "fork",
        VertexType::Join => "join",
        VertexType::Final => "final",
        VertexType::Terminate => "terminate",
        VertexType::Comment => "comment",
        VertexType::EntryPoint => "entry",
        VertexType::ExitPoint => "exit",
        _ => "",
    }
}

/// Whether a name is a generated one, not shown on the diagram.
pub fn is_generated(name: &str) -> bool {
    name.ends_with('*')
}

/// A name changed by `assign_names`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rename {
    pub what: String,
    pub from: String,
    pub to: String,
    /// Why, when it isn't the characters replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Clone, Copy)]
enum Named {
    MachineRegion(usize),
    Vertex(usize),
    VertexRegion(usize, usize),
}

fn get_name(m: &FsmModel, n: Named) -> &str {
    match n {
        Named::MachineRegion(i) => &m.regions[i].name,
        Named::Vertex(v) => &m.vertices[v].name,
        Named::VertexRegion(v, r) => &m.vertices[v].regions[r].name,
    }
}

fn set_name(m: &mut FsmModel, n: Named, name: String) {
    match n {
        Named::MachineRegion(i) => m.regions[i].name = name,
        Named::Vertex(v) => m.vertices[v].name = name,
        Named::VertexRegion(v, r) => m.vertices[v].regions[r].name = name,
    }
}

/// `s.trim().replace(/\s+/g, '_').replace(/[*.]/g, '_')`.
pub(crate) fn clean_name(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in js_trim(s).chars() {
        if js_space(c) {
            if !space {
                out.push('_');
            }
            space = true;
            continue;
        }
        space = false;
        out.push(if c == '*' || c == '.' { '_' } else { c });
    }
    out
}

/// Gives every element, point and region that needs a name one: a generated name (`choice1*`, smallest free number)
/// for those without. With `fix` (converting an old file), it also replaces spaces, `*` and `.` in names with `_`,
/// numbers the names used twice (`Idle2`), and renames points named `history` or `deepHistory`; it returns the
/// renames. Without it, a name used twice stays as it is (the validator reports it). Changes `model` in place.
pub fn assign_names(model: &mut FsmModel, fix: bool) -> Vec<Rename> {
    let clean = |s: &str| if fix { clean_name(s) } else { s.to_string() };
    let mut renames = Vec::new();
    let ix = Index::new(model);
    let mut taken: HashSet<String> = HashSet::new();
    let machine = clean(&model.name);
    if machine != model.name {
        renames.push(Rename {
            what: "the state machine".into(),
            from: model.name.clone(),
            to: machine.clone(),
            why: None,
        });
    }
    model.name = machine.clone();
    taken.insert(machine);
    // The named things in document order: regions of an owner with several, a single region with a name or a comment
    // attached to it (which needs a name to reference it), elements.
    let anchored: HashSet<String> = model.vertices.iter().flat_map(|v| v.anchors.iter().flatten().cloned()).collect();
    let single = |r: &Region| !r.name.is_empty() || anchored.contains(&r.id);
    let mut named: Vec<(Named, String, &'static str)> = Vec::new();
    if model.regions.len() >= 2 || (model.regions.len() == 1 && single(&model.regions[0])) {
        for i in 0..model.regions.len() {
            named.push((Named::MachineRegion(i), format!("region {} of the state machine", i + 1), "region"));
        }
    }
    for (vi, v) in model.vertices.iter().enumerate() {
        if !is_element(v.vtype) {
            continue;
        }
        named.push((Named::Vertex(vi), label(v.vtype).to_string(), generated_prefix(v.vtype)));
        if v.vtype == VertexType::State && (v.regions.len() >= 2 || (v.regions.len() == 1 && single(&v.regions[0]))) {
            let owner = if v.name.is_empty() { "a state".to_string() } else { format!("'{}'", v.name) };
            for ri in 0..v.regions.len() {
                named.push((Named::VertexRegion(vi, ri), format!("region {} of {owner}", ri + 1), "region"));
            }
        }
    }
    // Names kept first (the first of each), then the others.
    let mut later = Vec::new();
    for (n, what, prefix) in named {
        let old = get_name(model, n).to_string();
        let name = clean(&old);
        if !name.is_empty() && !taken.contains(&name) {
            if name != old {
                renames.push(Rename { what: format!("the {what}"), from: old, to: name.clone(), why: None });
            }
            set_name(model, n, name.clone());
            taken.insert(name);
        } else {
            later.push((n, what, prefix));
        }
    }
    let generate = |prefix: &str, used: &HashSet<String>| {
        let mut i = 1;
        loop {
            let n = format!("{prefix}{i}*");
            if !used.contains(&n) {
                return n;
            }
            i += 1;
        }
    };
    for (n, what, prefix) in later {
        let old = get_name(model, n).to_string();
        let base = clean(&old);
        let name = if base.is_empty() {
            generate(prefix, &taken)
        } else if !fix {
            continue;
        } else {
            let mut i = 2;
            while taken.contains(&format!("{base}{i}")) {
                i += 1;
            }
            let name = format!("{base}{i}");
            renames.push(Rename {
                what: format!("the {what}"),
                from: old,
                to: name.clone(),
                why: Some("its name was used twice".into()),
            });
            name
        };
        set_name(model, n, name.clone());
        taken.insert(name);
    }
    // Points: unique on their owner.
    let mut by_owner: Vec<(String, Vec<usize>)> = Vec::new();
    for (vi, v) in model.vertices.iter().enumerate() {
        if !(ix.is_border(v) || ix.is_machine_point(v)) || v.vtype == VertexType::ConnectionPointRef {
            continue;
        }
        let key = if ix.is_machine_point(v) { String::new() } else { v.parent.clone() };
        match by_owner.iter_mut().find(|(k, _)| *k == key) {
            Some((_, l)) => l.push(vi),
            None => by_owner.push((key, vec![vi])),
        }
    }
    for (_, points) in by_owner {
        let mut used: HashSet<String> =
            points.iter().map(|&p| model.vertices[p].name.clone()).filter(|n| !n.is_empty()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        for p in points {
            let old = model.vertices[p].name.clone();
            let vtype = model.vertices[p].vtype;
            let mut name = clean(&old);
            if fix && (name == "initial" || name == "history" || name == "deepHistory") {
                name.push('2');
            }
            if !name.is_empty() && !seen.contains(&name) {
                if name != old {
                    renames.push(Rename {
                        what: format!("the {}", label(vtype)),
                        from: old,
                        to: name.clone(),
                        why: None,
                    });
                }
            } else if name.is_empty() {
                name = generate(generated_prefix(vtype), &used);
            } else if fix {
                let mut i = 2;
                while seen.contains(&format!("{name}{i}")) || used.contains(&format!("{name}{i}")) {
                    i += 1;
                }
                renames.push(Rename {
                    what: format!("the {}", label(vtype)),
                    from: old,
                    to: format!("{name}{i}"),
                    why: Some("its name was used twice on its state".into()),
                });
                name = format!("{name}{i}");
            }
            model.vertices[p].name = name.clone();
            seen.insert(name.clone());
            used.insert(name);
        }
    }
    renames
}

// ------------------------------------------------------------------ layout

/// What reading the layout gives.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LayoutResult {
    /// Why the layout file couldn't be used (it is then left out entirely), or None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// How many elements had no layout entry and were placed automatically.
    pub placed: usize,
}

fn is_num(n: &Node) -> bool {
    n.number().is_some()
}

struct LayoutShape {
    problems: Vec<(usize, String)>,
}

impl LayoutShape {
    fn add(&mut self, line: usize, message: String) {
        self.problems.push((line, message));
    }

    fn check(
        &mut self,
        node: &Node,
        what: &str,
        allowed: &[&str],
        required: &[&str],
        each: &mut dyn FnMut(&mut Self, &Field),
    ) {
        let Some(fields) = node.fields() else {
            self.add(node.line, format!("{what} must be an object."));
            return;
        };
        for f in fields {
            if allowed.contains(&f.key.as_str()) {
                each(self, f);
            } else {
                self.add(f.line, format!("Unknown field '{}' in {what}.", f.key));
            }
        }
        for r in required {
            if !fields.iter().any(|f| r.split('|').any(|k| f.key == k)) {
                let r = r.split('|').next().unwrap_or(r);
                self.add(node.line, format!("Missing field '{r}' in {what}."));
            }
        }
        if fields.iter().any(|f| f.key == "regions") {
            for f in fields {
                if REGION_FIELDS.contains(&f.key.as_str()) {
                    self.add(f.line, format!("Field '{}' cannot be used with 'regions'.", f.key));
                }
            }
        }
    }

    fn point(&mut self, n: &Node, what: &str) {
        self.check(n, what, &["x", "y"], &["x", "y"], &mut |s, f| {
            if !is_num(&f.value) {
                s.add(f.value.line, format!("Field '{}' must be a number.", f.key));
            }
        });
    }

    fn list_of(&mut self, f: &Field, item: &mut dyn FnMut(&mut Self, &Node)) {
        match f.value.items() {
            None => self.add(f.value.line, format!("Field '{}' must be a list.", f.key)),
            Some(items) => {
                for n in items {
                    item(self, n);
                }
            }
        }
    }

    fn common(&mut self, f: &Field) {
        match f.key.as_str() {
            "x" | "y" | "width" | "height" | "size" => {
                if !is_num(&f.value) {
                    self.add(f.value.line, format!("Field '{}' must be a number.", f.key));
                }
            }
            "name" | "trigger" | "target" | "group" => {
                if f.value.str().is_none() {
                    self.add(f.value.line, format!("Field '{}' must be a text.", f.key));
                }
            }
            "color" => {
                if !f.value.str().is_some_and(is_color) {
                    self.add(f.value.line, "Field 'color' must be a color (#rrggbb).".into());
                }
            }
            "itemColors" => {
                let keys: Vec<&str> = ITEM_COLOR_KEYS.iter().map(|k| item_color_file_key(k)).collect();
                self.check(&f.value, "'itemColors'", &keys, &[], &mut |s, g| {
                    if !g.value.str().is_some_and(is_color) {
                        s.add(g.value.line, format!("Field '{}' must be a color (#rrggbb).", g.key));
                    }
                });
            }
            "label" => self.point(&f.value, "'label'"),
            "bends" => self.list_of(f, &mut |s, n| s.point(n, "a bend point")),
            "regionLayout" => {
                if !matches!(f.value.str(), Some("horizontal" | "vertical")) {
                    self.add(f.value.line, "Field 'regionLayout' must be \"horizontal\" or \"vertical\".".into());
                }
            }
            "transitions" => self.list_of(f, &mut |s, n| {
                s.check(
                    n,
                    "a transition",
                    &["trigger", "target", "bends", "label", "color", "itemColors"],
                    &[],
                    &mut |s, g| s.common(g),
                )
            }),
            "points" => self.list_of(f, &mut |s, n| {
                s.check(
                    n,
                    "a point",
                    &["name", "x", "y", "color", "itemColors", "group", "transitions"],
                    &["name"],
                    &mut |s, g| s.common(g),
                )
            }),
            "initial" | "history" | "deepHistory" => {
                let what = format!("'{}'", f.key);
                self.check(
                    &f.value,
                    &what,
                    &["x", "y", "group", "bends", "label", "color", "itemColors"],
                    &[],
                    &mut |s, g| s.common(g),
                );
            }
            "elements" => self.list_of(f, &mut |s, n| {
                s.check(
                    n,
                    "an element",
                    &[
                        "name",
                        "x",
                        "y",
                        "width",
                        "height",
                        "color",
                        "itemColors",
                        "group",
                        "regionLayout",
                        "transitions",
                        "points",
                        "regions",
                        "initial",
                        "history",
                        "deepHistory",
                        "elements",
                    ],
                    &["name"],
                    &mut |s, g| s.common(g),
                )
            }),
            "regions" => self.list_of(f, &mut |s, n| {
                s.check(
                    n,
                    "a region",
                    &["name", "size", "initial", "history", "deepHistory", "elements"],
                    &["name"],
                    &mut |s, g| s.common(g),
                )
            }),
            _ => {}
        }
    }
}

/// The first problem of the layout file's shape (enough for the warning), or None.
pub fn layout_shape_problem(root: &Node) -> Option<String> {
    if root.fields().is_none() {
        return Some("The layout file must hold a JSON object.".into());
    }
    let mut s = LayoutShape { problems: Vec::new() };
    s.check(
        root,
        "the layout",
        &["$schema", "version", "format", "points", "regions", "initial", "history", "deepHistory", "elements"],
        &["version|format"],
        &mut |s, f| {
            // `format`: the field's name before `version`, read the same way, never written.
            if f.key == "version" || f.key == "format" {
                if f.value.number() != Some(1.0) {
                    s.add(f.value.line, format!("Field '{}' must be 1.", f.key));
                }
            } else if f.key == "$schema" {
                if f.value.str().is_none() {
                    s.add(f.value.line, "Field '$schema' must be a text.".into());
                }
            } else {
                s.common(f);
            }
        },
    );
    s.problems.first().map(|(line, m)| format!("line {line}: {m}"))
}

struct Layout<'a> {
    model: &'a mut FsmModel,
    ix: Index,
    rel: HashMap<usize, (f64, f64)>,
    placed_v: HashSet<usize>,
    routes: Vec<(usize, &'a Node)>,
}

impl<'a> Layout<'a> {
    fn route(&mut self, ti: usize, e: &'a Node) {
        match self.routes.iter_mut().find(|(t, _)| *t == ti) {
            Some(r) => r.1 = e,
            None => self.routes.push((ti, e)),
        }
    }

    /// The place, size, colors and group of an entry; `colors` false for an initial or history entry, whose are its
    /// arrow's.
    fn set_box(&mut self, vi: usize, entry: &Node, with_colors: bool) {
        if let (Some(x), Some(y)) = (get_num(entry, "x"), get_num(entry, "y")) {
            self.rel.insert(vi, (x, y));
            self.placed_v.insert(vi);
        }
        let v = &mut self.model.vertices[vi];
        let (dw, dh) = default_size(v.vtype);
        let w = get_num(entry, "width");
        let h = get_num(entry, "height");
        v.w = match w {
            Some(w) if resizable(v.vtype) && w > 0.0 => w,
            _ => dw,
        };
        v.h = match h {
            Some(h) if resizable(v.vtype) && h > 0.0 => h,
            _ => dh,
        };
        if with_colors {
            if let Some(c) = get_str(entry, "color").filter(|c| is_color(c)) {
                v.color = Some(c.to_string());
            }
            if let Some(items) = entry.get("itemColors").and_then(|f| f.value.fields()) {
                let ic = item_colors_of(items);
                if !ic.is_empty() {
                    v.item_colors = Some(ic);
                }
            }
        }
        if let Some(g) = get_str(entry, "group").filter(|g| !g.is_empty()) {
            v.group = Some(g.to_string());
        }
    }

    /// Matches transition entries to the transitions leaving `source`: by trigger and target, then by order.
    fn match_transitions(&mut self, source: &str, entries: &'a [Node]) {
        let list: Vec<usize> =
            (0..self.model.transitions.len()).filter(|&i| self.model.transitions[i].source == source).collect();
        let mut used = HashSet::new();
        for e in entries {
            // The trigger as the model file may write it (`a,b`): the events it names.
            let trigger = split_events(get_str(e, "trigger").unwrap_or("")).join(", ");
            let target = get_str(e, "target");
            let found = list.iter().copied().find(|&i| {
                let x = &self.model.transitions[i];
                !used.contains(&i)
                    && x.triggers.join(", ") == trigger
                    && if x.kind == TransitionKind::Internal {
                        target.is_none()
                    } else {
                        self.ix.reference(&x.target).as_deref() == target
                    }
            });
            let Some(ti) = found else { continue };
            used.insert(ti);
            self.route(ti, e);
        }
    }

    fn apply_points(&mut self, owner: Option<usize>, node: &'a Node) {
        let points: Vec<usize> = (0..self.model.vertices.len())
            .filter(|&i| {
                let p = &self.ix.m.vertices[i];
                match owner {
                    Some(o) => p.parent == self.ix.m.vertices[o].id && self.ix.is_border(p),
                    None => self.ix.is_machine_point(p),
                }
            })
            .collect();
        let mut used = HashSet::new();
        for e in list(node, "points") {
            let name = get_str(e, "name");
            let Some(pi) = points
                .iter()
                .copied()
                .find(|&i| !used.contains(&i) && Some(Index::point_name(&self.ix.m.vertices[i]).as_str()) == name)
            else {
                continue;
            };
            used.insert(pi);
            self.set_box(pi, e, true);
            let id = self.model.vertices[pi].id.clone();
            self.match_transitions(&id, list(e, "transitions"));
        }
    }

    fn apply_region(&mut self, region_id: &str, node: &'a Node) {
        let kids: Vec<usize> = (0..self.model.vertices.len())
            .filter(|&i| {
                let v = &self.ix.m.vertices[i];
                v.parent == region_id && !self.ix.is_machine_point(v) && !self.ix.is_border(v)
            })
            .collect();
        for (key, vtype) in PSEUDO_FIELDS {
            let Some(e) = node.get(key).map(|f| &f.value) else { continue };
            let Some(vi) = kids.iter().copied().find(|&i| self.model.vertices[i].vtype == vtype) else { continue };
            self.set_box(vi, e, false);
            let id = self.model.vertices[vi].id.clone();
            if let Some(ti) = self.model.transitions.iter().position(|x| x.source == id) {
                self.route(ti, e);
            }
        }
        let mut used = HashSet::new();
        for e in list(node, "elements") {
            let name = get_str(e, "name");
            let Some(vi) = kids.iter().copied().find(|&i| {
                let x = &self.model.vertices[i];
                !used.contains(&i) && is_element(x.vtype) && Some(x.name.as_str()) == name
            }) else {
                continue;
            };
            used.insert(vi);
            self.set_box(vi, e, true);
            let id = self.model.vertices[vi].id.clone();
            if self.model.vertices[vi].vtype != VertexType::State {
                self.match_transitions(&id, list(e, "transitions"));
                continue;
            }
            match get_str(e, "regionLayout") {
                Some("horizontal") => self.model.vertices[vi].region_layout = Some(RegionLayout::Horizontal),
                Some("vertical") => self.model.vertices[vi].region_layout = Some(RegionLayout::Vertical),
                _ => {}
            }
            self.match_transitions(&id, list(e, "transitions"));
            self.apply_points(Some(vi), e);
            let regs: Vec<String> = self.model.vertices[vi].regions.iter().map(|r| r.id.clone()).collect();
            let items = e.get("regions").and_then(|f| f.value.items());
            if regs.len() == 1 && items.is_none() {
                self.apply_region(&regs[0], e);
            } else if let Some(items) = items {
                for item in items {
                    let iname = get_str(item, "name");
                    let Some(ri) = self.model.vertices[vi].regions.iter().position(|x| Some(x.name.as_str()) == iname)
                    else {
                        continue;
                    };
                    if let Some(size) = get_num(item, "size").filter(|&s| s > 0.0) {
                        self.model.vertices[vi].regions[ri].size = Some(size);
                    }
                    let rid = self.model.vertices[vi].regions[ri].id.clone();
                    self.apply_region(&rid, item);
                }
            }
        }
    }

    fn abs(&mut self, vi: usize, visiting: &mut HashSet<usize>) -> (f64, f64) {
        let v = &self.model.vertices[vi];
        if !v.x.is_nan() {
            return (v.x, v.y);
        }
        if !visiting.insert(vi) {
            return (0.0, 0.0);
        }
        let owner = self.ix.owner(&self.ix.m.vertices[vi]);
        let o = match owner {
            Some(oi) => self.abs(oi, visiting),
            None => (0.0, 0.0),
        };
        let p = self.rel.get(&vi).copied().unwrap_or((0.0, 0.0));
        let v = &mut self.model.vertices[vi];
        v.x = n1(o.0 + p.0);
        v.y = n1(o.1 + p.1);
        (v.x, v.y)
    }
}

fn item_colors_of(fields: &[Field]) -> ItemColors {
    let mut ic = ItemColors::new();
    for f in fields {
        if let (Some(k), Some(c)) = (item_color_from_file(&f.key), f.value.str().filter(|c| is_color(c))) {
            ic.insert(k.to_string(), c.to_string());
        }
    }
    ic
}

/// Gives the vertices of a model just read (`read_model`) their places and the transitions their routes, from the
/// layout file's text (None when there is none). Elements without an entry are placed automatically: default size,
/// the first free spot beside the elements already in their region, their state growing to fit. Entries that match
/// nothing are left out. The model's absolute coordinates are rounded to 0.1 px.
pub fn apply_layout(model: &mut FsmModel, layout_text: Option<&str>) -> LayoutResult {
    let mut result = LayoutResult::default();
    let mut root: Option<Node> = None;
    if let Some(t) = layout_text {
        match json::parse(t) {
            Ok(n) => match layout_shape_problem(&n) {
                Some(p) => result.problem = Some(p),
                None => root = Some(n),
            },
            Err(e) => result.problem = Some(format!("line {}: {}", e.line, e.message)),
        }
    }
    // The index's model keeps the structure; places and sizes are read from and written to `l.model`.
    let ix = Index::new(model);
    let mut l = Layout { model, ix, rel: HashMap::new(), placed_v: HashSet::new(), routes: Vec::new() };
    if let Some(root) = &root {
        l.apply_points(None, root);
        let items = root.get("regions").and_then(|f| f.value.items());
        let top: Vec<Region> = l.model.regions.clone();
        if top.len() == 1 && items.is_none() {
            l.apply_region(&top[0].id, root);
        } else if let Some(items) = items {
            for item in items {
                if let Some(r) = top.iter().find(|x| Some(x.name.as_str()) == get_str(item, "name")) {
                    l.apply_region(&r.id, item);
                }
            }
        }
    }
    let routes = std::mem::take(&mut l.routes);

    // Automatic places, region by region, innermost first: a state grows to hold what was placed in it automatically
    // (a state that grew is in turn new to its own region) before the elements beside it are placed.
    let mut auto: HashSet<usize> = HashSet::new();
    for v in l.model.vertices.iter_mut() {
        if v.w == 0.0 || v.w.is_nan() || v.h == 0.0 || v.h.is_nan() {
            (v.w, v.h) = default_size(v.vtype);
        }
    }
    let n = l.model.vertices.len();
    let mut region_ids: Vec<String> = l.model.regions.iter().map(|r| r.id.clone()).collect();
    for v in &l.model.vertices {
        for r in &v.regions {
            region_ids.push(r.id.clone());
        }
    }
    region_ids.reverse();
    for rid in region_ids {
        let owner = l.ix.region_owner.get(&rid).copied().flatten();
        let kids: Vec<usize> =
            (0..n).filter(|&i| l.model.vertices[i].parent == rid && !l.ix.is_border(&l.ix.m.vertices[i])).collect();
        let index: i64 = match owner {
            Some(o) => l.model.vertices[o].regions.iter().position(|r| r.id == rid).map_or(-1, |p| p as i64),
            None => 0,
        };
        for &vi in &kids {
            if l.placed_v.contains(&vi) {
                continue;
            }
            let placed: Vec<(f64, f64, f64, f64)> = kids
                .iter()
                .filter(|k| l.placed_v.contains(k))
                .map(|&k| {
                    let p = l.rel[&k];
                    (p.0, p.1, l.model.vertices[k].w, l.model.vertices[k].h)
                })
                .collect();
            let p = if !placed.is_empty() {
                (
                    placed.iter().fold(f64::NEG_INFINITY, |m, b| js_max(m, b.0 + b.2)) + 40.0,
                    placed.iter().fold(f64::INFINITY, |m, b| js_min(m, b.1)),
                )
            } else if owner.is_some() {
                (30.0, 50.0 + (index.max(0) as f64) * 80.0)
            } else {
                (40.0, 40.0)
            };
            l.rel.insert(vi, (n1(p.0), n1(p.1)));
            l.placed_v.insert(vi);
            auto.insert(vi);
            result.placed += 1;
        }
        if let Some(o) = owner
            && index == 0
            && l.model.vertices[o].vtype == VertexType::State
        {
            // grow(owner)
            let ids: HashSet<String> = l.model.vertices[o].regions.iter().map(|r| r.id.clone()).collect();
            let kids: Vec<usize> = (0..n)
                .filter(|&k| ids.contains(&l.model.vertices[k].parent) && !l.ix.is_border(&l.ix.m.vertices[k]))
                .collect();
            if kids.iter().any(|k| auto.contains(k)) {
                let before = (l.model.vertices[o].w, l.model.vertices[o].h);
                for k in kids {
                    let p = l.rel.get(&k).copied().unwrap_or((f64::NAN, f64::NAN));
                    let (kw, kh) = (l.model.vertices[k].w, l.model.vertices[k].h);
                    let v = &mut l.model.vertices[o];
                    v.w = js_max(v.w, n1(p.0 + kw + 20.0));
                    v.h = js_max(v.h, n1(p.1 + kh + 20.0));
                }
                let v = &l.model.vertices[o];
                if v.w != before.0 || v.h != before.1 {
                    auto.insert(o);
                }
            }
        }
    }
    // Points without a place: on their state's border, entries on the left, exits on the right.
    for vi in 0..n {
        let v = &l.ix.m.vertices[vi];
        if !l.ix.is_border(v) || l.placed_v.contains(&vi) {
            continue;
        }
        let Some(&oi) = l.ix.v.get(&v.parent) else { continue };
        let owner_id = l.model.vertices[oi].id.clone();
        let same: Vec<usize> = (0..n)
            .filter(|&p| {
                let x = &l.ix.m.vertices[p];
                x.parent == owner_id && l.ix.is_border(x) && !l.placed_v.contains(&p) && x.vtype == v.vtype
            })
            .collect();
        let i = same.iter().position(|&p| p == vi).map_or(-1.0, |p| p as f64);
        let exit = v.vtype == VertexType::ExitPoint
            || (v.vtype == VertexType::ConnectionPointRef && v.point_kind == Some(PointKind::Exit));
        let (ow, oh) = (l.model.vertices[oi].w, l.model.vertices[oi].h);
        let (vw, vh) = (l.model.vertices[vi].w, l.model.vertices[vi].h);
        l.rel.insert(vi, (n1(if exit { ow - vw / 2.0 } else { -vw / 2.0 }), n1(js_min(oh - vh, 10.0 + i * 24.0))));
        result.placed += 1;
    }
    // Absolute coordinates, owners first.
    for vi in 0..n {
        let mut visiting = HashSet::new();
        l.abs(vi, &mut visiting);
    }
    // Routes
    for (ti, e) in routes {
        let f = l.ix.frame(&l.ix.m.transitions[ti]);
        let o = f.map_or((0.0, 0.0), |fi| (l.model.vertices[fi].x, l.model.vertices[fi].y));
        let t = &mut l.model.transitions[ti];
        if let Some(bends) = e.get("bends").and_then(|f| f.value.items()).filter(|b| !b.is_empty())
            && t.kind != TransitionKind::Internal
        {
            t.points = Some(
                bends
                    .iter()
                    .map(|b| Point {
                        x: n1(o.0 + get_num(b, "x").unwrap_or(f64::NAN)),
                        y: n1(o.1 + get_num(b, "y").unwrap_or(f64::NAN)),
                    })
                    .collect(),
            );
        }
        if let Some(label) = e.get("label").map(|f| &f.value)
            && t.kind != TransitionKind::Internal
        {
            t.label_offset =
                Some(Point { x: get_num(label, "x").unwrap_or(f64::NAN), y: get_num(label, "y").unwrap_or(f64::NAN) });
        }
        if let Some(c) = get_str(e, "color").filter(|c| is_color(c)) {
            t.color = Some(c.to_string());
        }
        if let Some(items) = e.get("itemColors").and_then(|f| f.value.fields()) {
            let ic = item_colors_of(items);
            if !ic.is_empty() {
                t.item_colors = Some(ic);
            }
        }
    }
    normalize_groups(l.model);
    result
}

/// Reads a machine's two files.
pub fn read_machine(model_text: &str, layout_text: Option<&str>) -> Result<(ReadMachine, LayoutResult), Vec<Problem>> {
    let mut read = read_model(model_text)?;
    let layout = apply_layout(&mut read.model, layout_text);
    Ok((read, layout))
}

/// Resolves the references of `model` that named nothing when they were read (a transition's target, a comment's
/// anchors: UNRESOLVED…) and name something now, as reading its file again would (`written`: `write_machine(model)`).
/// Changes `model` in place; returns whether it changed.
pub fn resolve_references(model: &mut FsmModel, written: &Written) -> bool {
    let open = |a: &str| a.starts_with(UNRESOLVED) || a.starts_with(UNRESOLVED_ANCHOR);
    let pending = model.transitions.iter().any(|t| unresolved(&t.target).is_some_and(|u| !u.is_empty()))
        || model.vertices.iter().any(|v| v.anchors.iter().flatten().any(|a| open(a)));
    if !pending {
        return false;
    }
    let Ok(read) = read_model(&written.model) else { return false };
    let ids = derived_ids(model, Some(written));
    let own: HashMap<&String, &String> = ids.iter().map(|(id, d)| (d, id)).collect();
    let rt: HashMap<&String, &Transition> = read.model.transitions.iter().map(|t| (&t.id, t)).collect();
    let rv: HashMap<&String, &Vertex> = read.model.vertices.iter().map(|v| (&v.id, v)).collect();
    let mut changed = false;
    for t in &mut model.transitions {
        if !unresolved(&t.target).is_some_and(|u| !u.is_empty()) {
            continue;
        }
        let target = ids.get(&t.id).and_then(|d| rt.get(d)).map(|x| &x.target);
        if let Some(id) = target.and_then(|tg| own.get(tg)) {
            t.target = (*id).clone();
            changed = true;
        }
    }
    for v in &mut model.vertices {
        let Some(anchors) = &v.anchors else { continue };
        if !anchors.iter().any(|a| open(a)) {
            continue;
        }
        let read_anchors: Vec<String> =
            ids.get(&v.id).and_then(|d| rv.get(d)).and_then(|x| x.anchors.clone()).unwrap_or_default();
        let new: Vec<String> = anchors
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let id = if open(a) { read_anchors.get(i).and_then(|r| own.get(r)) } else { None };
                match id {
                    Some(id) => {
                        changed = true;
                        (*id).clone()
                    }
                    None => a.clone(),
                }
            })
            .collect();
        v.anchors = Some(new);
    }
    changed
}

/// The ids reading `model` back from its file would give (what fsm-cli calls the elements, in generated code's traces
/// and in its problems), by the model's own ids: vertices and transitions.
pub fn derived_ids(model: &FsmModel, written: Option<&Written>) -> HashMap<String, String> {
    let owned;
    let written = match written {
        Some(w) => w,
        None => {
            owned = write_machine(model);
            &owned
        }
    };
    let mut out = HashMap::new();
    let Ok(read) = read_model(&written.model) else { return out };
    let mut by_line: HashMap<usize, String> = HashMap::new();
    let mut by_line_t: HashMap<usize, String> = HashMap::new();
    for v in &read.model.vertices {
        by_line.insert(read.lines[&v.id], v.id.clone());
    }
    for t in &read.model.transitions {
        by_line_t.insert(read.lines[&t.id], t.id.clone());
    }
    for v in &model.vertices {
        if let Some(id) = written.lines.get(&v.id).and_then(|l| by_line.get(l)) {
            out.insert(v.id.clone(), id.clone());
        }
    }
    for t in &model.transitions {
        if let Some(id) = written.lines.get(&t.id).and_then(|l| by_line_t.get(l)) {
            out.insert(t.id.clone(), id.clone());
        }
    }
    out
}
