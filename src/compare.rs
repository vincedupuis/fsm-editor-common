//! Comparing two versions of a machine, for the diagram diff (two panes) and merge (three panes).
//!
//! Elements are matched by what identifies them in the file: an element by its name, a region of `regions` by its
//! state and name, a point by its state and name, an initial or history pseudostate by its region, a transition by
//! its source, trigger and target (then by order among identical ones; a transition left over is then paired with one
//! of the same source and trigger, else of the same source, in order, its target or trigger having changed). A renamed
//! element is one removed and one added.
//!
//! Each match lists the properties that differ, model and layout apart: positions are compared relative to the owner
//! (moving a state doesn't change what it contains), bend points relative to the innermost state containing both ends.

use std::collections::{HashMap, HashSet};

use fsm_cli::json;
use serde::Serialize;

use crate::format::{UNRESOLVED, UNRESOLVED_ANCHOR};
use crate::model::{
    FsmModel, ITEM_COLOR_KEYS, ItemColors, MachineKind, PointKind, RegionLayout, Transition, TransitionKind, Vertex,
    VertexType, n1, s,
};

/// A property that differs, with its two values as text ('' when absent).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PropertyChange {
    pub name: String,
    pub old: String,
    pub new: String,
    /// A property of the drawing, not of the model.
    pub layout: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// What identifies the element in the newer version (see `items_of`).
    pub key: String,
    /// Its key in the older version, when it differs (a transition whose target or trigger changed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_key: Option<String>,
    /// `added`, `removed` or `changed`.
    pub kind: &'static str,
    /// `machine`, `state`, `choice`…, `region`, `transition`.
    pub what: String,
    /// How a person names it: `state 'Off'`, `transition Off -> On (powerOn)`.
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_id: Option<String>,
    /// For `changed`: what differs.
    pub properties: Vec<PropertyChange>,
}

/// Fields in order (a JavaScript object's).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Props(pub Vec<(String, String)>);

impl Props {
    fn set(&mut self, k: &str, v: String) {
        match self.0.iter_mut().find(|(n, _)| n == k) {
            Some(x) => x.1 = v,
            None => self.0.push((k.to_string(), v)),
        }
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

/// What can be compared in a model: its elements by key, with their properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: String,
    pub id: String,
    pub what: String,
    pub label: String,
    pub model: Props,
    pub layout: Props,
}

fn num(n: Option<f64>) -> String {
    match n {
        Some(n) if n.is_finite() && n1(n).is_finite() => json::number(n1(n)),
        _ => String::new(),
    }
}

fn what_of(t: VertexType) -> &'static str {
    match t {
        VertexType::ShallowHistory => "history",
        VertexType::DeepHistory => "deep history",
        VertexType::EntryPoint => "entry point",
        VertexType::ExitPoint => "exit point",
        VertexType::ConnectionPointRef => "point",
        VertexType::Final => "final state",
        VertexType::Terminate => "terminate",
        other => other.key(),
    }
}

fn shown(name: &str) -> &str {
    if name.ends_with('*') { "" } else { name }
}

fn colors(color: &Option<String>, items: &Option<ItemColors>, into: &mut Props) {
    into.set("color", s(color).to_string());
    for k in ITEM_COLOR_KEYS {
        if let Some(c) = items.as_ref().and_then(|i| i.get(k)).filter(|c| !c.is_empty()) {
            into.set(&format!("color of {k}"), c.clone());
        }
    }
}

struct Ix<'a> {
    m: &'a FsmModel,
    v: HashMap<&'a str, usize>,
    region_owner: HashMap<&'a str, Option<usize>>,
    region_names: HashMap<&'a str, &'a str>,
    top: HashSet<&'a str>,
    keys: HashMap<&'a str, String>,
}

impl<'a> Ix<'a> {
    fn is_border(&self, v: &Vertex) -> bool {
        v.vtype == VertexType::ConnectionPointRef
            || (matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint)
                && self.v.contains_key(v.parent.as_str()))
    }

    fn is_machine_point(&self, v: &Vertex) -> bool {
        matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint) && self.top.contains(v.parent.as_str())
    }

    fn owner(&self, v: &Vertex) -> Option<usize> {
        if self.is_border(v) {
            self.v.get(v.parent.as_str()).copied()
        } else {
            self.region_owner.get(v.parent.as_str()).copied().flatten()
        }
    }

    fn vx(&self, i: usize) -> &'a Vertex {
        &self.m.vertices[i]
    }

    /// A region's key: its owner and name, or index for a state's only region.
    fn region_key(&self, rid: &str) -> String {
        let o = self.region_owner.get(rid).copied().flatten();
        let len = match o {
            Some(o) => self.vx(o).regions.len(),
            None => self.m.regions.len(),
        };
        let at =
            if len > 1 { format!(":{}", self.region_names.get(rid).copied().unwrap_or("")) } else { String::new() };
        match o {
            Some(o) => format!("{}{at}", self.vx(o).name),
            None => at,
        }
    }

    fn vertex_key(&self, v: &Vertex) -> String {
        if matches!(v.vtype, VertexType::Initial | VertexType::ShallowHistory | VertexType::DeepHistory) {
            return format!("{}@{}", v.vtype.key(), self.region_key(&v.parent));
        }
        if self.is_border(v) {
            let owner = self.v.get(v.parent.as_str()).map(|&i| self.vx(i).name.as_str()).unwrap_or("");
            let name = if v.vtype == VertexType::ConnectionPointRef { s(&v.point_ref) } else { &v.name };
            return format!("point@{owner}.{name}");
        }
        if self.is_machine_point(v) {
            return format!("point@.{}", v.name);
        }
        format!("element@{}", v.name)
    }

    fn ref_of(&self, id: &str) -> String {
        if !self.v.contains_key(id) {
            return id.strip_prefix(UNRESOLVED).unwrap_or(id).to_string();
        }
        let k = &self.keys[id];
        if let Some(r) = k.strip_prefix("element@") {
            r.to_string()
        } else if let Some(r) = k.strip_prefix("point@.") {
            format!("{}.{r}", self.m.name)
        } else if let Some(r) = k.strip_prefix("point@") {
            r.to_string()
        } else {
            k.clone()
        }
    }

    fn name_of(v: &'a Vertex) -> &'a str {
        shown(if v.vtype == VertexType::ConnectionPointRef { s(&v.point_ref) } else { &v.name })
    }

    /// Where a vertex is: its state, and the region of a state (or machine) with several.
    fn place_of(&self, v: &Vertex) -> String {
        let o = self.owner(v);
        if self.is_border(v) {
            return o.map(|o| format!("'{}'", shown(&self.vx(o).name))).unwrap_or_default();
        }
        let list = match o {
            Some(o) => &self.vx(o).regions,
            None => &self.m.regions,
        };
        let i = list.iter().position(|r| r.id == v.parent);
        let region = if list.len() > 1 {
            let name = i.map(|i| shown(&list[i].name)).unwrap_or("");
            let index = i.map_or(0, |i| i + 1);
            format!("region '{}'", if name.is_empty() { index.to_string() } else { name.to_string() })
        } else {
            String::new()
        };
        match o {
            Some(o) => {
                let n = format!("'{}'", shown(&self.vx(o).name));
                if region.is_empty() { n } else { format!("{n}, {region}") }
            }
            None => region,
        }
    }

    fn label_of(&self, v: &'a Vertex) -> String {
        let what = what_of(v.vtype);
        let n = Ix::name_of(v);
        if !n.is_empty() {
            return format!("{what} '{n}'");
        }
        match self.owner(v).map(|o| shown(&self.vx(o).name)).filter(|n| !n.is_empty()) {
            Some(o) => format!("{what} in '{o}'"),
            None => what.to_string(),
        }
    }

    fn chain(&self, id: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.v.get(id).copied();
        if let Some(i) = cur
            && self.vx(i).vtype != VertexType::State
        {
            cur = self.owner(self.vx(i));
        }
        let mut seen = HashSet::new();
        while let Some(i) = cur {
            if !seen.insert(i) {
                break;
            }
            out.push(i);
            cur = self.owner(self.vx(i));
        }
        out
    }

    fn frame(&self, t: &Transition) -> Option<usize> {
        let g: HashSet<usize> = self.chain(&t.target).into_iter().collect();
        self.chain(&t.source).into_iter().find(|v| g.contains(v))
    }
}

/// The items of a model, keyed so that the same element has the same key in another version, in order.
pub fn items_of(model: &FsmModel) -> Vec<Item> {
    let mut ix = Ix {
        m: model,
        v: HashMap::new(),
        region_owner: HashMap::new(),
        region_names: HashMap::new(),
        top: model.regions.iter().map(|r| r.id.as_str()).collect(),
        keys: HashMap::new(),
    };
    for (i, v) in model.vertices.iter().enumerate() {
        ix.v.entry(&v.id).or_insert(i);
    }
    for r in &model.regions {
        ix.region_owner.insert(&r.id, None);
        ix.region_names.insert(&r.id, &r.name);
    }
    for (i, v) in model.vertices.iter().enumerate() {
        for r in &v.regions {
            ix.region_owner.insert(&r.id, Some(i));
            ix.region_names.insert(&r.id, &r.name);
        }
    }
    let keys: Vec<(&str, String)> = model.vertices.iter().map(|v| (v.id.as_str(), ix.vertex_key(v))).collect();
    for (id, k) in keys {
        ix.keys.insert(id, k);
    }
    let t_by_id: HashMap<&str, &Transition> = model.transitions.iter().map(|t| (t.id.as_str(), t)).collect();
    let region_names: HashMap<&str, &str> = model
        .regions
        .iter()
        .chain(model.vertices.iter().flat_map(|v| v.regions.iter()))
        .map(|r| (r.id.as_str(), r.name.as_str()))
        .collect();
    let anchor_of = |a: &str| -> String {
        if a == crate::format::MACHINE_ANCHOR {
            return model.name.clone();
        }
        if ix.v.contains_key(a) {
            return ix.ref_of(a);
        }
        if let Some(name) = region_names.get(a) {
            return name.to_string();
        }
        let Some(t) = t_by_id.get(a) else {
            return a.strip_prefix([UNRESOLVED, UNRESOLVED_ANCHOR]).unwrap_or(a).to_string();
        };
        let trigger = t.triggers.join(", ");
        let target = if t.kind == TransitionKind::Internal { "(internal)".to_string() } else { ix.ref_of(&t.target) };
        format!(
            "{} -> {target}{}",
            ix.ref_of(&t.source),
            if trigger.is_empty() { String::new() } else { format!(" ({trigger})") }
        )
    };

    let mut items: Vec<Item> = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut machine = Props::default();
    machine.set("name", model.name.clone());
    machine.set("type", if model.kind == MachineKind::Protocol { "protocol" } else { "behavioral" }.into());
    machine.set("context", s(&model.context).to_string());
    machine.set("documentation", s(&model.documentation).to_string());
    items.push(Item {
        key: "machine".into(),
        id: String::new(),
        what: "machine".into(),
        label: "the state machine".into(),
        model: machine,
        layout: Props::default(),
    });
    seen_keys.insert("machine".into());
    for v in &model.vertices {
        let key = ix.keys[v.id.as_str()].clone();
        if !seen_keys.insert(key.clone()) {
            continue; // a name used twice: the first
        }
        let o = ix.owner(v);
        let mut m = Props::default();
        m.set("type", v.vtype.key().into());
        m.set("in", ix.place_of(v));
        if v.vtype == VertexType::State {
            m.set("entry", s(&v.entry).into());
            m.set("exit", s(&v.exit).into());
            m.set("do", s(&v.do_activity).into());
            m.set("defer", v.deferrable.clone().unwrap_or_default().join(", "));
            m.set("invariant", s(&v.invariant).into());
            m.set("stereotype", s(&v.stereotype).into());
            m.set("submachine", s(&v.submachine).into());
            m.set(
                "regions",
                if v.regions.len() > 1 || v.regions.iter().any(|r| !r.name.is_empty()) {
                    v.regions
                        .iter()
                        .map(|r| if shown(&r.name).is_empty() { "(unnamed)" } else { shown(&r.name) })
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    v.regions.len().to_string()
                },
            );
        }
        if v.vtype == VertexType::Comment {
            m.set("text", s(&v.text).into());
            m.set("anchors", v.anchors.iter().flatten().map(|a| anchor_of(a)).collect::<Vec<_>>().join(", "));
        }
        if v.vtype == VertexType::ConnectionPointRef {
            m.set("direction", if v.point_kind == Some(PointKind::Exit) { "exit" } else { "entry" }.into());
        }
        // Where it is, relative to its owner.
        let (ox, oy) = o.map_or((0.0, 0.0), |o| (ix.vx(o).x, ix.vx(o).y));
        let mut l = Props::default();
        l.set("x", num(Some(v.x - ox)));
        l.set("y", num(Some(v.y - oy)));
        l.set("width", num(Some(v.w)));
        l.set("height", num(Some(v.h)));
        l.set("group", if v.group.as_deref().is_some_and(|g| !g.is_empty()) { "grouped" } else { "" }.into());
        if v.regions.len() > 1 {
            l.set(
                "region layout",
                if v.region_layout == Some(RegionLayout::Horizontal) { "horizontal" } else { "vertical" }.into(),
            );
            for (i, r) in v.regions.iter().enumerate() {
                let name = if shown(&r.name).is_empty() { (i + 1).to_string() } else { shown(&r.name).to_string() };
                l.set(&format!("size of region {name}"), num(r.size));
            }
        }
        colors(&v.color, &v.item_colors, &mut l);
        items.push(Item {
            key,
            id: v.id.clone(),
            what: what_of(v.vtype).to_string(),
            label: ix.label_of(v),
            model: m,
            layout: l,
        });
    }
    // Transitions: by source, trigger and target, then order.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for t in &model.transitions {
        let sv = ix.v.get(t.source.as_str()).map(|&i| ix.vx(i));
        let source = match sv {
            Some(sv) => ix.keys[sv.id.as_str()].clone(),
            None => format!("?{}", t.source),
        };
        let trigger = t.triggers.join(", ");
        let target = if t.kind == TransitionKind::Internal { String::new() } else { ix.ref_of(&t.target) };
        let base = format!("transition@{source}|{trigger}|{target}");
        let n = seen.get(&base).copied().unwrap_or(0);
        seen.insert(base.clone(), n + 1);
        let key = format!("{base}|{n}");
        let sname = match sv {
            Some(sv) => {
                if shown(&sv.name).is_empty() {
                    ix.label_of(sv)
                } else {
                    shown(&sv.name).to_string()
                }
            }
            None => t.source.clone(),
        };
        let tname = if t.kind == TransitionKind::Internal {
            "(internal)".to_string()
        } else {
            target.strip_suffix('*').unwrap_or(&target).to_string()
        };
        let label = format!(
            "transition {sname} -> {tname}{}",
            if trigger.is_empty() { String::new() } else { format!(" ({trigger})") }
        );
        let (fx, fy) = ix.frame(t).map_or((0.0, 0.0), |f| (ix.vx(f).x, ix.vx(f).y));
        let mut l = Props::default();
        l.set(
            "bend points",
            t.points
                .iter()
                .flatten()
                .map(|p| format!("{},{}", num(Some(p.x - fx)), num(Some(p.y - fy))))
                .collect::<Vec<_>>()
                .join(" "),
        );
        l.set("label", t.label_offset.map(|o| format!("{},{}", num(Some(o.x)), num(Some(o.y)))).unwrap_or_default());
        colors(&t.color, &t.item_colors, &mut l);
        let mut m = Props::default();
        m.set("kind", t.kind.key().into());
        m.set("trigger", trigger.clone());
        m.set("guard", t.guard.clone());
        m.set("action", t.action.clone());
        m.set("precondition", s(&t.precondition).into());
        m.set("postcondition", s(&t.postcondition).into());
        m.set("target", target);
        m.set("source", source);
        items.push(Item { key, id: t.id.clone(), what: "transition".into(), label, model: m, layout: l });
    }
    items
}

/// The properties of two items that differ.
fn differences(a: &Item, b: &Item) -> Vec<PropertyChange> {
    let mut out = Vec::new();
    for (layout, x, y) in [(false, &a.model, &b.model), (true, &a.layout, &b.layout)] {
        let mut names: Vec<&str> = x.0.iter().map(|(k, _)| k.as_str()).collect();
        for (k, _) in &y.0 {
            if !names.contains(&k.as_str()) {
                names.push(k);
            }
        }
        for k in names {
            if k == "source" {
                continue;
            }
            let (o, n) = (x.get(k).unwrap_or(""), y.get(k).unwrap_or(""));
            if o != n {
                out.push(PropertyChange { name: k.to_string(), old: o.to_string(), new: n.to_string(), layout });
            }
        }
    }
    out
}

/// What changed from `older` to `newer`, in the order of the newer version (removed ones come last).
pub fn compare_models(older: &FsmModel, newer: &FsmModel) -> Vec<Change> {
    let a = items_of(older);
    let b = items_of(newer);
    let a_by: HashMap<&str, &Item> = a.iter().map(|i| (i.key.as_str(), i)).collect();
    let mut pairs: HashMap<String, String> = HashMap::new(); // new key -> old key
    for i in &b {
        if a_by.contains_key(i.key.as_str()) {
            pairs.insert(i.key.clone(), i.key.clone());
        }
    }
    // Transitions left over: the same source and trigger, else the same source, in order.
    let left_a: Vec<&Item> = a.iter().filter(|i| i.what == "transition" && !pairs.contains_key(&i.key)).collect();
    let left_b: Vec<&Item> = b.iter().filter(|i| i.what == "transition" && !pairs.contains_key(&i.key)).collect();
    let part = |k: &str, i: usize| k.split('|').nth(i).unwrap_or("").to_string();
    let mut used: HashSet<String> = HashSet::new();
    for level in [2, 1] {
        for t in &left_b {
            if pairs.contains_key(&t.key) {
                continue;
            }
            let (src, trig) = (part(&t.key, 0), part(&t.key, 1));
            let found = left_a
                .iter()
                .find(|x| !used.contains(&x.key) && part(&x.key, 0) == src && (level == 1 || part(&x.key, 1) == trig));
            if let Some(o) = found {
                pairs.insert(t.key.clone(), o.key.clone());
                used.insert(o.key.clone());
            }
        }
    }
    let matched_old: HashSet<&String> = pairs.values().collect();
    let mut changes = Vec::new();
    for item in &b {
        let Some(old) = pairs.get(&item.key) else {
            changes.push(Change {
                key: item.key.clone(),
                old_key: None,
                kind: "added",
                what: item.what.clone(),
                label: item.label.clone(),
                old_id: None,
                new_id: Some(item.id.clone()),
                properties: Vec::new(),
            });
            continue;
        };
        let o = a_by[old.as_str()];
        let props = differences(o, item);
        if !props.is_empty() {
            changes.push(Change {
                key: item.key.clone(),
                old_key: if *old != item.key { Some(old.clone()) } else { None },
                kind: "changed",
                what: item.what.clone(),
                label: item.label.clone(),
                old_id: Some(o.id.clone()),
                new_id: Some(item.id.clone()),
                properties: props,
            });
        }
    }
    for item in &a {
        if !matched_old.contains(&item.key) {
            changes.push(Change {
                key: item.key.clone(),
                old_key: None,
                kind: "removed",
                what: item.what.clone(),
                label: item.label.clone(),
                old_id: Some(item.id.clone()),
                new_id: None,
                properties: Vec::new(),
            });
        }
    }
    changes
}

/// Whether a change shows when layout changes are hidden.
pub fn is_model_change(c: &Change) -> bool {
    c.kind != "changed" || c.properties.iter().any(|p| !p.layout)
}

/// A change's properties as lines: `guard: a() → b()`.
pub fn describe_change(c: &Change, layout: bool) -> Vec<String> {
    let value = |v: &str| -> String {
        let units: Vec<u16> = v.encode_utf16().collect();
        if v.is_empty() {
            "(none)".into()
        } else if units.len() > 60 {
            format!("{}…", String::from_utf16_lossy(&units[..57]))
        } else {
            v.to_string()
        }
    };
    c.properties
        .iter()
        .filter(|p| layout || !p.layout)
        .map(|p| format!("{}: {} → {}", p.name, value(&p.old), value(&p.new)))
        .collect()
}
