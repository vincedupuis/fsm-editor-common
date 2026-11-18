//! The diagram merge (three panes): splitting a file with conflict markers into its two sides, and taking one side's
//! version of an element into the result.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::compare::items_of;
use crate::format::{UNRESOLVED, reference_of, unresolved_anchor};
use crate::model::{
    FsmModel, Point, Region, Transition, TransitionKind, Vertex, VertexType, js_round, normalize_model,
};

/// Whether a text holds Git's conflict markers.
pub fn has_conflict_markers(text: &str) -> bool {
    let marker = |c: char| {
        text.split('\n').any(|line| {
            let rest = line.strip_prefix(&c.to_string().repeat(7));
            matches!(rest, Some(r) if r.starts_with(' ') || r.is_empty() || r == "\r")
        })
    };
    marker('<') && marker('>')
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Split {
    /// The text with the current side of every conflict ("ours").
    pub current: String,
    /// The text with the incoming side of every conflict ("theirs").
    pub incoming: String,
    /// How many conflicts.
    pub conflicts: usize,
}

/// The marker a line starts with: seven of `<`, `|`, `=` or `>`, followed by a space or the line's end.
fn marker(line: &str) -> Option<char> {
    for c in ['<', '|', '=', '>'] {
        if let Some(rest) = line.strip_prefix(&c.to_string().repeat(7))
            && (rest.starts_with(' ') || rest.is_empty() || rest == "\n" || rest == "\r" || rest == "\r\n")
        {
            return Some(c);
        }
    }
    None
}

/// Splits a text with conflict markers into its two sides: `<<<<<<<` current `=======` incoming `>>>>>>>`, and the
/// three-part style, whose base section (`|||||||`) is left out. The error is the message when the markers don't
/// nest.
pub fn split_conflicts(text: &str) -> Result<Split, String> {
    #[derive(PartialEq)]
    enum State {
        Both,
        Current,
        Base,
        Incoming,
    }
    let mut current = String::new();
    let mut incoming = String::new();
    let mut state = State::Both;
    let mut conflicts = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let at = i + 1;
        match marker(line) {
            Some('<') => {
                if state != State::Both {
                    return Err(format!("Line {at}: a conflict starts inside another one."));
                }
                state = State::Current;
                conflicts += 1;
            }
            Some('|') if state == State::Current => state = State::Base,
            Some('=') if state == State::Current || state == State::Base => state = State::Incoming,
            Some('>') => {
                if state != State::Incoming {
                    return Err(format!("Line {at}: a conflict ends where none is open."));
                }
                state = State::Both;
            }
            _ => match state {
                State::Both => {
                    current.push_str(line);
                    incoming.push_str(line);
                }
                State::Current => current.push_str(line),
                State::Incoming => incoming.push_str(line),
                State::Base => {}
            },
        }
    }
    if state != State::Both {
        return Err("The last conflict has no end marker (>>>>>>>).".into());
    }
    Ok(Split { current, incoming, conflicts })
}

/// Owners in a model: vertices by id (the last of each, as a JavaScript Map built from the list).
struct Owners {
    v: HashMap<String, usize>,
    region_owner: HashMap<String, Option<usize>>,
}

impl Owners {
    fn new(m: &FsmModel) -> Owners {
        let mut o = Owners { v: HashMap::new(), region_owner: HashMap::new() };
        for (i, v) in m.vertices.iter().enumerate() {
            o.v.insert(v.id.clone(), i);
        }
        for r in &m.regions {
            o.region_owner.insert(r.id.clone(), None);
        }
        for (i, v) in m.vertices.iter().enumerate() {
            for r in &v.regions {
                o.region_owner.insert(r.id.clone(), Some(i));
            }
        }
        o
    }

    fn border(&self, v: &Vertex) -> bool {
        v.vtype == VertexType::ConnectionPointRef
            || (matches!(v.vtype, VertexType::EntryPoint | VertexType::ExitPoint) && self.v.contains_key(&v.parent))
    }

    fn owner(&self, v: &Vertex) -> Option<usize> {
        if self.border(v) {
            self.v.get(&v.parent).copied()
        } else {
            self.region_owner.get(&v.parent).copied().flatten()
        }
    }

    /// Whether vertex `vi` is inside vertex `si` (at any depth).
    fn inside(&self, m: &FsmModel, vi: usize, si: usize) -> bool {
        let mut seen = HashSet::new();
        let mut o = self.owner(&m.vertices[vi]);
        while let Some(i) = o {
            if !seen.insert(i) {
                return false;
            }
            if i == si {
                return true;
            }
            o = self.owner(&m.vertices[i]);
        }
        false
    }
}

struct Adopt<'a> {
    r: FsmModel,
    source: &'a FsmModel,
    so: Owners,
    taken: HashSet<String>,
    key_of_s: HashMap<String, String>,
    id_of_r: HashMap<String, String>,
}

fn r1(n: f64) -> f64 {
    js_round(n * 10.0) / 10.0
}

impl Adopt<'_> {
    fn fresh(&mut self, base: &str) -> String {
        let mut id = format!("{base}~m");
        let mut n = 2;
        while self.taken.contains(&id) {
            id = format!("{base}~m{n}");
            n += 1;
        }
        self.taken.insert(id.clone());
        id
    }

    /// A source element's key; one without (a name used twice) shares one key, as `undefined` does in a Map.
    fn key_of(&self, sid: &str) -> String {
        self.key_of_s.get(sid).cloned().unwrap_or_else(|| "\0undefined".into())
    }

    fn r_id_of_s(&self, sid: &str) -> Option<String> {
        self.key_of_s.get(sid).and_then(|k| self.id_of_r.get(k)).cloned()
    }

    /// The result's parent for a source vertex: the same region of the same owner, else the top-level region.
    fn parent_in(&self, si: usize) -> String {
        let ro = Owners::new(&self.r);
        let v = &self.source.vertices[si];
        if self.so.border(v) {
            return self.r_id_of_s(&v.parent).unwrap_or_default();
        }
        let Some(owner) = self.so.owner(v).map(|o| &self.source.vertices[o]) else {
            let index = self.source.regions.iter().position(|x| x.id == v.parent);
            return index.and_then(|i| self.r.regions.get(i)).unwrap_or(&self.r.regions[0]).id.clone();
        };
        let o = self.r_id_of_s(&owner.id).and_then(|id| ro.v.get(&id).copied()).map(|i| &self.r.vertices[i]);
        let Some(o) = o.filter(|o| !o.regions.is_empty()) else { return self.r.regions[0].id.clone() };
        let index = owner.regions.iter().position(|x| x.id == v.parent);
        let wanted = index.map(|i| owner.regions[i].name.as_str());
        let named = o.regions.iter().find(|x| !x.name.is_empty() && Some(x.name.as_str()) == wanted);
        named.or_else(|| index.and_then(|i| o.regions.get(i))).unwrap_or(&o.regions[0]).id.clone()
    }

    /// Places result vertex `ti` where source vertex `si` is, relative to its owner.
    fn place(&mut self, ti: usize, si: usize) {
        let ro = Owners::new(&self.r);
        let v = &self.source.vertices[si];
        let a = self.so.owner(v).map_or((0.0, 0.0), |o| (self.source.vertices[o].x, self.source.vertices[o].y));
        let parent = &self.r.vertices[ti].parent;
        let r_owner = ro.v.get(parent).copied().or_else(|| ro.region_owner.get(parent).copied().flatten());
        let b = match r_owner {
            Some(o) if o != ti => (self.r.vertices[o].x, self.r.vertices[o].y),
            _ => (0.0, 0.0),
        };
        let (vx, vy) = (v.x, v.y);
        let t = &mut self.r.vertices[ti];
        t.x = r1(b.0 + vx - a.0);
        t.y = r1(b.1 + vy - a.1);
    }

    fn copy_vertex(&mut self, ti: usize, si: usize) {
        let v = &self.source.vertices[si];
        let regions: Vec<(Region, Option<String>)> = v
            .regions
            .iter()
            .enumerate()
            .map(|(i, x)| (x.clone(), self.r.vertices[ti].regions.get(i).map(|y| y.id.clone())))
            .collect();
        let regs: Vec<Region> = regions
            .into_iter()
            .map(|(x, id)| {
                let id = id.unwrap_or_else(|| self.fresh(&x.id));
                Region { id, ..x }
            })
            .collect();
        // An anchor to what the result doesn't have (yet) is kept as the reference the file writes, which names it
        // once it is taken too.
        let anchors = v.anchors.as_ref().map(|list| {
            list.iter()
                .map(|a| {
                    self.r_id_of_s(a)
                        .or_else(
                            || if self.key_of_s.contains_key(a) { unresolved_anchor(self.source, a) } else { None },
                        )
                        .unwrap_or_else(|| a.clone())
                })
                .collect::<Vec<_>>()
        });
        let t = &mut self.r.vertices[ti];
        t.vtype = v.vtype;
        t.name = v.name.clone();
        t.w = v.w;
        t.h = v.h;
        t.region_layout = v.region_layout;
        t.entry = v.entry.clone();
        t.exit = v.exit.clone();
        t.do_activity = v.do_activity.clone();
        t.deferrable = v.deferrable.clone();
        t.invariant = v.invariant.clone();
        t.submachine = v.submachine.clone();
        t.stereotype = v.stereotype.clone();
        t.point_ref = v.point_ref.clone();
        t.point_kind = v.point_kind;
        t.text = v.text.clone();
        t.color = v.color.clone();
        t.item_colors = v.item_colors.clone();
        t.group = v.group.clone();
        t.regions = regs;
        t.anchors = anchors;
    }

    fn remove_vertex(&mut self, id: &str) {
        let ro = Owners::new(&self.r);
        let Some(&vi) = ro.v.get(id) else { return };
        let mut gone: HashSet<String> = HashSet::new();
        gone.insert(self.r.vertices[vi].id.clone());
        for xi in 0..self.r.vertices.len() {
            if ro.inside(&self.r, xi, vi) {
                gone.insert(self.r.vertices[xi].id.clone());
            }
        }
        self.r.vertices.retain(|x| !gone.contains(&x.id));
        self.r.transitions.retain(|t| !gone.contains(&t.source) && !gone.contains(&t.target));
        for x in &mut self.r.vertices {
            if let Some(a) = &mut x.anchors {
                a.retain(|a| !gone.contains(a));
            }
        }
    }

    fn add_vertex(&mut self, si: usize) {
        let v = &self.source.vertices[si];
        let id = self.fresh(&v.id);
        let parent = self.parent_in(si);
        let mut n = Vertex::new(&id, v.vtype, &v.name, &parent);
        (n.x, n.y, n.w, n.h) = (0.0, 0.0, v.w, v.h);
        self.r.vertices.push(n);
        let ti = self.r.vertices.len() - 1;
        self.copy_vertex(ti, si);
        let k = self.key_of(&self.source.vertices[si].id);
        self.id_of_r.insert(k, id);
        self.place(ti, si);
        // What it contains comes along.
        for xi in 0..self.source.vertices.len() {
            if xi == si || !self.so.inside(self.source, xi, si) {
                continue;
            }
            let k = self.key_of(&self.source.vertices[xi].id);
            if !self.id_of_r.contains_key(&k) {
                self.add_vertex(xi);
            }
        }
    }

    /// The target in the result; one the result doesn't have (yet) is the reference the file writes, naming nothing.
    fn target_in(&self, t: &Transition) -> String {
        if let Some(id) = self.r_id_of_s(&t.target) {
            return id;
        }
        let known = self.source.vertices.iter().any(|v| v.id == t.target);
        match if known { reference_of(self.source, &t.target) } else { None } {
            Some(r) => format!("{UNRESOLVED}{r}"),
            None => t.target.clone(),
        }
    }

    fn copy_transition(&mut self, ti: usize, t: &Transition) {
        let target =
            if t.kind == TransitionKind::Internal { self.r.transitions[ti].source.clone() } else { self.target_in(t) };
        // Bend points, moved with the difference between the two sources' places.
        let s1 = self.source.vertices.iter().rev().find(|x| x.id == t.source);
        let s2 = self.r.vertices.iter().find(|x| x.id == self.r.transitions[ti].source);
        let points = match (&t.points, s1, s2) {
            (Some(p), Some(s1), Some(s2)) => {
                Some(p.iter().map(|p| Point { x: r1(p.x - s1.x + s2.x), y: r1(p.y - s1.y + s2.y) }).collect::<Vec<_>>())
            }
            _ => None,
        };
        let x = &mut self.r.transitions[ti];
        x.kind = t.kind;
        x.triggers = t.triggers.clone();
        x.guard = t.guard.clone();
        x.action = t.action.clone();
        x.precondition = t.precondition.clone();
        x.postcondition = t.postcondition.clone();
        x.color = t.color.clone();
        x.item_colors = t.item_colors.clone();
        x.label_offset = t.label_offset;
        x.target = target;
        x.points = points;
    }
}

/// The result with the element of `keys` (see compare.rs: its key in either version, for a transition whose target
/// or trigger changed) as `source` has it: updated, added (with what it contains), or removed (with what it contains
/// and the transitions to and from it). Positions are taken relative to the owner. The result's own ids stay; added
/// elements get new ones.
pub fn adopt(result: &FsmModel, source: &FsmModel, keys: &[String]) -> FsmModel {
    let mut r = result.clone();
    let key = keys.first().map(String::as_str).unwrap_or("");
    if key == "machine" {
        r.name = source.name.clone();
        r.kind = source.kind;
        r.documentation = source.documentation.clone();
        normalize_model(&mut r);
        return r;
    }
    let ri = items_of(&r);
    let si = items_of(source);
    let mut taken: HashSet<String> = HashSet::new();
    taken.extend(r.vertices.iter().map(|v| v.id.clone()));
    taken.extend(r.transitions.iter().map(|t| t.id.clone()));
    taken.extend(r.regions.iter().map(|x| x.id.clone()));
    taken.extend(r.vertices.iter().flat_map(|v| v.regions.iter().map(|x| x.id.clone())));
    // Keys of the source's elements, by id, and the result's ids, by key.
    let mut key_of_s = HashMap::new();
    for it in &si {
        if !it.id.is_empty() {
            key_of_s.insert(it.id.clone(), it.key.clone());
        }
    }
    let mut id_of_r = HashMap::new();
    for it in &ri {
        if !it.id.is_empty() {
            id_of_r.insert(it.key.clone(), it.id.clone());
        }
    }
    let s_item = keys.iter().find_map(|k| si.iter().find(|i| i.key == *k)).cloned();
    let mine = keys.iter().find_map(|k| ri.iter().find(|i| i.key == *k)).cloned();
    let mut a = Adopt { r, source, so: Owners::new(source), taken, key_of_s, id_of_r };
    let is_t = |i: &Option<crate::compare::Item>| i.as_ref().is_some_and(|i| i.what == "transition");
    if is_t(&s_item) || is_t(&mine) {
        match &s_item {
            None => {
                let id = mine.as_ref().unwrap().id.clone();
                a.r.transitions.retain(|t| t.id != id);
            }
            Some(s) => {
                let t = source.transitions.iter().rev().find(|t| t.id == s.id).unwrap().clone();
                let existing = mine.as_ref().and_then(|m| a.r.transitions.iter().position(|x| x.id == m.id));
                match existing {
                    Some(ti) => a.copy_transition(ti, &t),
                    None => {
                        if let Some(source2) = a.r_id_of_s(&t.source) {
                            let id = a.fresh(&t.id);
                            a.r.transitions.push(Transition::new(&id, &source2, "", t.kind));
                            let ti = a.r.transitions.len() - 1;
                            a.copy_transition(ti, &t);
                        }
                    }
                }
            }
        }
        normalize_model(&mut a.r);
        return a.r;
    }
    let Some(s) = s_item else {
        if let Some(m) = mine {
            a.remove_vertex(&m.id);
        }
        normalize_model(&mut a.r);
        return a.r;
    };
    let si_index = a.so.v[&s.id];
    match mine.as_ref().and_then(|m| a.r.vertices.iter().position(|x| x.id == m.id)) {
        Some(ti) => {
            let parent = a.parent_in(si_index);
            if !parent.is_empty() {
                a.r.vertices[ti].parent = parent;
            }
            let (x0, y0) = (a.r.vertices[ti].x, a.r.vertices[ti].y);
            a.copy_vertex(ti, si_index);
            a.place(ti, si_index);
            // What it contains moves with it, bend points between them included.
            let dx = a.r.vertices[ti].x - x0;
            let dy = a.r.vertices[ti].y - y0;
            let moved = |d: f64| d != 0.0 && !d.is_nan();
            if moved(dx) || moved(dy) {
                let ro = Owners::new(&a.r);
                let mut inner: HashSet<String> = (0..a.r.vertices.len())
                    .filter(|&xi| ro.inside(&a.r, xi, ti))
                    .map(|xi| a.r.vertices[xi].id.clone())
                    .collect();
                for x in &mut a.r.vertices {
                    if inner.contains(&x.id) {
                        x.x += dx;
                        x.y += dy;
                    }
                }
                inner.insert(a.r.vertices[ti].id.clone());
                for t in &mut a.r.transitions {
                    if let Some(p) = &mut t.points
                        && inner.contains(&t.source)
                        && inner.contains(&t.target)
                    {
                        for q in p.iter_mut() {
                            q.x += dx;
                            q.y += dy;
                        }
                    }
                }
            }
        }
        None => a.add_vertex(si_index),
    }
    normalize_model(&mut a.r);
    a.r
}
