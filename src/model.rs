//! The editor's model: a flat UML state machine with absolute coordinates, as the three editions work on it. Its JSON
//! form (camelCase fields, optional ones left out) is the model the editions exchange with this library.
//!
//! The hierarchy is flat: every vertex points to its owner through `parent`, which is either a region id (a region
//! of the machine or of a composite state) or, for the points on a state's border, the id of that state. Entry/exit
//! points whose parent is a top-level region are the machine's own connection points.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VertexType {
    State,
    Final,
    Initial,
    ShallowHistory,
    DeepHistory,
    Choice,
    Junction,
    Fork,
    Join,
    EntryPoint,
    ExitPoint,
    Terminate,
    ConnectionPointRef,
    Comment,
}

impl VertexType {
    pub fn key(self) -> &'static str {
        match self {
            VertexType::State => "state",
            VertexType::Final => "final",
            VertexType::Initial => "initial",
            VertexType::ShallowHistory => "shallowHistory",
            VertexType::DeepHistory => "deepHistory",
            VertexType::Choice => "choice",
            VertexType::Junction => "junction",
            VertexType::Fork => "fork",
            VertexType::Join => "join",
            VertexType::EntryPoint => "entryPoint",
            VertexType::ExitPoint => "exitPoint",
            VertexType::Terminate => "terminate",
            VertexType::ConnectionPointRef => "connectionPointRef",
            VertexType::Comment => "comment",
        }
    }

    pub fn from_key(s: &str) -> Option<VertexType> {
        Some(match s {
            "state" => VertexType::State,
            "final" => VertexType::Final,
            "initial" => VertexType::Initial,
            "shallowHistory" => VertexType::ShallowHistory,
            "deepHistory" => VertexType::DeepHistory,
            "choice" => VertexType::Choice,
            "junction" => VertexType::Junction,
            "fork" => VertexType::Fork,
            "join" => VertexType::Join,
            "entryPoint" => VertexType::EntryPoint,
            "exitPoint" => VertexType::ExitPoint,
            "terminate" => VertexType::Terminate,
            "connectionPointRef" => VertexType::ConnectionPointRef,
            "comment" => VertexType::Comment,
            _ => return None,
        })
    }

    pub fn is_pseudostate(self) -> bool {
        !matches!(self, VertexType::State | VertexType::Final | VertexType::ConnectionPointRef | VertexType::Comment)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PointKind {
    Entry,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransitionKind {
    #[default]
    External,
    Internal,
    Local,
}

impl TransitionKind {
    pub fn key(self) -> &'static str {
        match self {
            TransitionKind::External => "external",
            TransitionKind::Internal => "internal",
            TransitionKind::Local => "local",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineKind {
    #[default]
    Behavioral,
    Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegionLayout {
    Horizontal,
    Vertical,
}

/// Items of a state or transition that can have their own color, in file order: `background` is the fill of a
/// composite state's regions, the others are items of the text.
pub const ITEM_COLOR_KEYS: [&str; 11] = [
    "background",
    "entry",
    "exit",
    "doActivity",
    "deferrable",
    "invariant",
    "triggers",
    "guard",
    "action",
    "precondition",
    "postcondition",
];

/// Diagram colors per item, `#rrggbb`, by key of ITEM_COLOR_KEYS.
pub type ItemColors = BTreeMap<String, String>;

/// The color value accepted in files and in the editor.
pub fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A number of the model's JSON: `null` (what JSON makes of a number that isn't finite) reads as 0, as JavaScript
/// reads it in arithmetic.
fn num<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(0.0))
}

fn opt_num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Option::<f64>::deserialize(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    #[serde(deserialize_with = "num", default)]
    pub x: f64,
    #[serde(deserialize_with = "num", default)]
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Region {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Regions of a state: extent along the state's region layout. The last region takes what the others leave.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "opt_num")]
    pub size: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vertex {
    pub id: String,
    #[serde(rename = "type")]
    pub vtype: VertexType,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub parent: String,
    #[serde(deserialize_with = "num", default)]
    pub x: f64,
    #[serde(deserialize_with = "num", default)]
    pub y: f64,
    #[serde(deserialize_with = "num", default)]
    pub w: f64,
    #[serde(deserialize_with = "num", default)]
    pub h: f64,
    #[serde(default)]
    pub regions: Vec<Region>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_layout: Option<RegionLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub do_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferrable: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant: Option<String>,
    /// Submachine state: path of the referenced machine's file, relative to this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submachine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stereotype: Option<String>,
    /// A connection point reference: the name of the point in the submachine's file.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub point_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_kind: Option<PointKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Ids of the annotated vertices and transitions (references that name nothing: see format.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_colors: Option<ItemColors>,
    /// The group the vertex belongs to: its members move together (see normalize_groups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl Vertex {
    /// A vertex just read, not placed yet (`x` is NaN).
    pub fn new(id: &str, vtype: VertexType, name: &str, parent: &str) -> Vertex {
        Vertex {
            id: id.to_string(),
            vtype,
            name: name.to_string(),
            parent: parent.to_string(),
            x: f64::NAN,
            y: f64::NAN,
            w: 0.0,
            h: 0.0,
            regions: Vec::new(),
            region_layout: None,
            entry: None,
            exit: None,
            do_activity: None,
            deferrable: None,
            invariant: None,
            submachine: None,
            stereotype: None,
            point_ref: None,
            point_kind: None,
            text: None,
            anchors: None,
            color: None,
            item_colors: None,
            group: None,
        }
    }

    pub fn regions_len(&self) -> usize {
        self.regions.len()
    }
}

/// An optional text, `''` when absent (JavaScript's `x ?? ''`).
pub fn s(o: &Option<String>) -> &str {
    o.as_deref().unwrap_or("")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub id: String,
    pub source: String,
    /// The target vertex's id, or a reference that names nothing (see `unresolved` in format.rs).
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub kind: TransitionKind,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub guard: String,
    #[serde(default)]
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precondition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postcondition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Point>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_offset: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_colors: Option<ItemColors>,
}

impl Transition {
    pub fn new(id: &str, source: &str, target: &str, kind: TransitionKind) -> Transition {
        Transition {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            kind,
            triggers: Vec::new(),
            guard: String::new(),
            action: String::new(),
            precondition: None,
            postcondition: None,
            points: None,
            label_offset: None,
            color: None,
            item_colors: None,
        }
    }
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FsmModel {
    #[serde(default = "one")]
    pub version: u8,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: MachineKind,
    /// The name of the classifier the machine belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default)]
    pub regions: Vec<Region>,
    #[serde(default)]
    pub vertices: Vec<Vertex>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

impl FsmModel {
    pub fn new(name: &str) -> FsmModel {
        FsmModel {
            version: 1,
            name: name.to_string(),
            kind: MachineKind::Behavioral,
            context: None,
            documentation: None,
            regions: Vec::new(),
            vertices: Vec::new(),
            transitions: Vec::new(),
        }
    }
}

/// Vertices drawn on the border of a state (entry/exit points and connection point references).
pub fn is_border_vertex(v: &Vertex, vertex_ids: &HashSet<&str>) -> bool {
    v.vtype == VertexType::ConnectionPointRef
        || (matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint) && vertex_ids.contains(v.parent.as_str()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionPointInfo {
    /// The point's name, by which the machines using it refer to it.
    pub id: String,
    pub name: String,
    pub kind: PointKind,
}

/// Entry/exit points owned by the machine itself (placed in a top-level region).
pub fn machine_connection_points(model: &FsmModel) -> Vec<ConnectionPointInfo> {
    let top: HashSet<&str> = model.regions.iter().map(|r| r.id.as_str()).collect();
    model
        .vertices
        .iter()
        .filter(|v| {
            matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint) && top.contains(v.parent.as_str())
        })
        .map(|v| ConnectionPointInfo {
            id: v.name.clone(),
            name: v.name.clone(),
            kind: if v.vtype == VertexType::EntryPoint { PointKind::Entry } else { PointKind::Exit },
        })
        .collect()
}

/// Fills in defaults, so that a model sent by an editor is safe to use.
pub fn normalize_model(m: &mut FsmModel) {
    m.version = 1;
    if m.regions.is_empty() {
        m.regions = vec![Region { id: "r_root".into(), name: String::new(), size: None }];
    }
    normalize_groups(m);
    let types: HashMap<String, VertexType> = m.vertices.iter().map(|v| (v.id.clone(), v.vtype)).collect();
    for t in &mut m.transitions {
        // A state's self-transition is a loop whose bend points are its three corners, or only its far corner:
        // other numbers of free bend points, from older versions, go back to the default loop.
        let n = t.points.as_ref().map_or(0, Vec::len);
        if t.source == t.target
            && t.kind != TransitionKind::Internal
            && types.get(&t.source) == Some(&VertexType::State)
            && (n == 2 || n > 3)
        {
            t.points = None;
            t.label_offset = None;
        }
    }
}

/// Keeps the groups the diagram can show. A group is the vertices sharing a `group` id: those on a state's border
/// leave it, and so do those outside the region of its first member (in file order). A group left with fewer than
/// two members is dissolved.
pub fn normalize_groups(m: &mut FsmModel) {
    let ids: HashSet<String> = m.vertices.iter().map(|v| v.id.clone()).collect();
    let id_refs: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let border: Vec<bool> = m.vertices.iter().map(|v| is_border_vertex(v, &id_refs)).collect();
    let mut parents: HashMap<String, String> = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (i, v) in m.vertices.iter_mut().enumerate() {
        let Some(g) = v.group.clone().filter(|g| !g.is_empty()) else {
            v.group = None;
            continue;
        };
        if border[i] {
            v.group = None;
            continue;
        }
        match parents.get(&g) {
            None => {
                parents.insert(g.clone(), v.parent.clone());
            }
            Some(p) if *p != v.parent => {
                v.group = None;
                continue;
            }
            _ => {}
        }
        *counts.entry(g).or_insert(0) += 1;
    }
    for v in &mut m.vertices {
        if let Some(g) = &v.group
            && counts.get(g).copied().unwrap_or(0) < 2
        {
            v.group = None;
        }
    }
}

// ------------------------------------------------------------------ JavaScript's arithmetic

/// `Math.round`: halves toward +∞.
pub fn js_round(n: f64) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let r = (n + 0.5).floor();
    // (n + 0.5) can round up in floating point for the largest number below 0.5.
    if r - n > 0.5 { r - 1.0 } else { r }
}

/// A number rounded to 0.1, as the files write layout values.
pub fn n1(n: f64) -> f64 {
    js_round(n * 10.0) / 10.0
}

/// `Math.max` of two numbers: NaN when either is.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// `Math.min` of two numbers: NaN when either is.
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}
