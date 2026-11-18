//! Reading state machine files of earlier versions: XMI 2.5.1 holding a UML 2.5.1 model, plus a UML DI diagram
//! (umldi:UMLStateMachineDiagram) with the layout. Nothing writes this format any more: saving such a file writes
//! the JSON format (format.rs), converting it (convert.rs).
//!
//! Layout mapping:
//! - every vertex, region and comment has a UMLShape with dc:Bounds;
//! - every drawn transition has a UMLEdge whose waypoints are the centers of its source and target with the bend
//!   points in between, plus an optional UMLLabel whose center is the middle of those waypoints moved by the label
//!   offset;
//! - the region layout of an orthogonal state, and each region's size along it, follow from its region shapes;
//! - colors and groups are kept in one xmi:Extension at the end of the diagram, with a
//!   <style element="id" color="#rrggbb" background="#rrggbb" entry="#rrggbb" …/> per colored vertex or transition,
//!   then a <group id="g" members="id id …"/> per group of vertices.

use std::collections::HashMap;

use fsm_cli::model::{js_space, js_trim};

use crate::model::{
    FsmModel, ITEM_COLOR_KEYS, ItemColors, MachineKind, Point, PointKind, Region, RegionLayout, Transition,
    TransitionKind, Vertex, VertexType, is_color, normalize_model,
};
use crate::xml::{XmlElement, parse_xml};

pub const NS_XMI: &str = "http://www.omg.org/spec/XMI/20131001";
pub const NS_UML: &str = "http://www.omg.org/spec/UML/20161101";
pub const NS_UMLDI: &str = "http://www.omg.org/spec/UML/20161101/UMLDI";
pub const NS_DC: &str = "http://www.omg.org/spec/DD/20100524/DC";
pub const NS_DI: &str = "http://www.omg.org/spec/DD/20100524/DI";
const PROFILE_NS: &str = "https://github.com/vincedupuis/fsm-editor/profiles/FsmStereotypes";
const EXTENDER: &str = "FSM Editor";

/// A model read from an old file: the model, with what the JSON format has no place for.
#[derive(Debug, Clone)]
pub struct XmiModel {
    pub model: FsmModel,
    /// xmi:id of the state machine, which hrefs to it name (`Payment.fsm#sm`).
    pub xmi_id: String,
    /// Name of the context classifier, which the JSON format doesn't hold.
    pub context: String,
}

/// JavaScript's `Number(text)` (`+text`).
pub fn js_to_number(text: &str) -> f64 {
    let s = js_trim(text);
    if s.is_empty() {
        return 0.0;
    }
    let lower = &s[..s.len().min(2)];
    let radix = match lower {
        "0x" | "0X" => 16,
        "0o" | "0O" => 8,
        "0b" | "0B" => 2,
        _ => 0,
    };
    if radix != 0 {
        let digits = &s[2..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return f64::NAN;
        }
        return digits.chars().fold(0.0, |n, c| n * radix as f64 + c.to_digit(radix).unwrap() as f64);
    }
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    if body == "Infinity" {
        return if s.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // StrDecimalLiteral: digits [. digits] [e [+-] digits], or . digits [...]
    let b = body.as_bytes();
    let mut i = 0;
    let int = b.iter().take_while(|c| c.is_ascii_digit()).count();
    i += int;
    let mut frac = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        frac = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        i += frac;
    }
    if int == 0 && frac == 0 {
        return f64::NAN;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if exp == 0 {
            return f64::NAN;
        }
        i += exp;
    }
    if i != b.len() {
        return f64::NAN;
    }
    s.parse::<f64>().unwrap_or(f64::NAN)
}

/// `+text || 0`.
fn num0(text: Option<&str>) -> f64 {
    let n = num(text);
    if n.is_nan() || n == 0.0 { 0.0 } else { n }
}

/// `+text` (NaN for a missing attribute).
fn num(text: Option<&str>) -> f64 {
    match text {
        Some(t) => js_to_number(t),
        None => f64::NAN,
    }
}

fn type_of(el: &XmlElement) -> String {
    match el.attr("xmi:type") {
        Some(t) => t.to_string(),
        None if el.ns == NS_UML => format!("uml:{}", el.local),
        None => String::new(),
    }
}

fn id_of(el: &XmlElement) -> String {
    el.attr("xmi:id").unwrap_or("").to_string()
}

fn kids<'a>(el: &'a XmlElement, name: &'a str) -> impl Iterator<Item = &'a XmlElement> {
    el.children.iter().filter(move |c| c.name == name)
}

fn kid<'a>(el: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    el.children.iter().find(|c| c.name == name)
}

/// A reference property, written as an attribute (`a="id1 id2"`) or as child elements (`<a xmi:idref|href>`).
fn refs(el: &XmlElement, name: &str) -> Vec<String> {
    let mut out: Vec<String> =
        el.attr(name).unwrap_or("").split(js_space).filter(|s| !s.is_empty()).map(str::to_string).collect();
    for c in kids(el, name) {
        if let Some(r) = c.attr("xmi:idref").or_else(|| c.attr("href")).filter(|r| !r.is_empty()) {
            out.push(r.to_string());
        }
    }
    out
}

fn local_id(r: &str) -> String {
    match r.find('#') {
        Some(i) => r[i + 1..].to_string(),
        None => r.to_string(),
    }
}

/// Text of an opaque behavior or constraint (its first body).
fn body_of(el: Option<&XmlElement>, trim: bool) -> String {
    let Some(el) = el else { return String::new() };
    let spec = kid(el, "specification").unwrap_or(el);
    let text = match kid(spec, "body") {
        Some(b) => b.text.clone(),
        None => spec.attr("body").or_else(|| spec.attr("value")).unwrap_or("").to_string(),
    };
    if trim { js_trim(&text).to_string() } else { text }
}

fn walk<'a>(el: &'a XmlElement, out: &mut Vec<&'a XmlElement>) {
    out.push(el);
    for c in &el.children {
        walk(c, out);
    }
}

fn or_empty(a: Option<&str>) -> Option<&str> {
    a.filter(|s| !s.is_empty())
}

/// JavaScript's `Math.hypot` of two numbers (V8's: scaled by the larger, with a compensated sum), which can differ
/// from Rust's in the last bit: the label offsets of old files depend on it.
pub fn js_hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let max = a.max(b);
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for x in [a, b] {
        let r = x / max;
        let summand = r * r - compensation;
        let preliminary = sum + summand;
        compensation = preliminary - sum - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

fn polyline_mid(pts: &[Point]) -> Point {
    let mut total = 0.0;
    for i in 1..pts.len() {
        total += js_hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y);
    }
    let mut half = total / 2.0;
    for i in 1..pts.len() {
        let (a, b) = (pts[i - 1], pts[i]);
        let len = js_hypot(b.x - a.x, b.y - a.y);
        if len >= half && len > 0.0 {
            return Point { x: a.x + ((b.x - a.x) * half) / len, y: a.y + ((b.y - a.y) * half) / len };
        }
        half -= len;
    }
    pts[0]
}

fn default_size(t: VertexType) -> (f64, f64) {
    crate::format::default_size(t)
}

struct Reader<'a> {
    by_id: HashMap<String, &'a XmlElement>,
    model: FsmModel,
}

impl<'a> Reader<'a> {
    fn event_text(&self, trig: &XmlElement) -> String {
        let ev_ref = trig
            .attr("event")
            .map(str::to_string)
            .or_else(|| refs(trig, "event").into_iter().next())
            .unwrap_or_default();
        let ev = self.by_id.get(&local_id(&ev_ref)).copied();
        if let Some(ev) = ev {
            let t = type_of(ev);
            if t == "uml:TimeEvent" {
                let expr = kid(kid(ev, "when").unwrap_or(ev), "expr");
                let value = match expr {
                    Some(e) => e.attr("value").map(str::to_string).unwrap_or_else(|| body_of(Some(e), true)),
                    None => String::new(),
                };
                if !value.is_empty() {
                    return format!("after({value})");
                }
            }
            if t == "uml:SignalEvent" {
                let sig = self.by_id.get(&local_id(ev.attr("signal").unwrap_or(""))).copied();
                return or_empty(sig.and_then(|s| s.attr("name")))
                    .or_else(|| or_empty(ev.attr("name")))
                    .or_else(|| or_empty(trig.attr("name")))
                    .unwrap_or("")
                    .to_string();
            }
        }
        or_empty(ev.and_then(|e| e.attr("name"))).or_else(|| or_empty(trig.attr("name"))).unwrap_or("").to_string()
    }

    fn vertex_base(el: &XmlElement, vtype: VertexType, parent: &str) -> Vertex {
        Vertex::new(&id_of(el), vtype, el.attr("name").unwrap_or(""), parent)
    }

    fn read_transition(&mut self, el: &XmlElement) {
        let one =
            |k: &str| el.attr(k).map(str::to_string).or_else(|| refs(el, k).into_iter().next()).unwrap_or_default();
        let kind = match el.attr("kind") {
            Some("internal") => TransitionKind::Internal,
            Some("local") => TransitionKind::Local,
            _ => TransitionKind::External,
        };
        let mut t = Transition::new(&id_of(el), &local_id(&one("source")), &local_id(&one("target")), kind);
        t.triggers = kids(el, "trigger").map(|x| self.event_text(x)).filter(|s| !s.is_empty()).collect();
        t.guard = body_of(kid(el, "guard"), true);
        t.action = body_of(kid(el, "effect"), true);
        let pre = body_of(kid(el, "preCondition"), true);
        let post = body_of(kid(el, "postCondition"), true);
        if !pre.is_empty() {
            t.precondition = Some(pre);
        }
        if !post.is_empty() {
            t.postcondition = Some(post);
        }
        self.model.transitions.push(t);
    }

    fn read_point(&mut self, el: &XmlElement, parent: &str) {
        let kind = if el.attr("kind") == Some("exitPoint") { VertexType::ExitPoint } else { VertexType::EntryPoint };
        self.model.vertices.push(Reader::vertex_base(el, kind, parent));
    }

    fn read_vertex(&mut self, el: &XmlElement, parent: &str) {
        let t = type_of(el);
        if t == "uml:FinalState" {
            self.model.vertices.push(Reader::vertex_base(el, VertexType::Final, parent));
            return;
        }
        if t == "uml:Pseudostate" {
            let kind = VertexType::from_key(el.attr("kind").unwrap_or("initial"))
                .filter(|k| k.is_pseudostate())
                .unwrap_or(VertexType::Initial);
            self.model.vertices.push(Reader::vertex_base(el, kind, parent));
            return;
        }
        let mut v = Reader::vertex_base(el, VertexType::State, parent);
        let non_empty = |s: String| if s.is_empty() { None } else { Some(s) };
        v.entry = non_empty(body_of(kid(el, "entry"), true));
        v.exit = non_empty(body_of(kid(el, "exit"), true));
        v.do_activity = non_empty(body_of(kid(el, "doActivity"), true));
        v.invariant = non_empty(body_of(kid(el, "stateInvariant"), true));
        let deferrable: Vec<String> =
            kids(el, "deferrableTrigger").map(|x| self.event_text(x)).filter(|s| !s.is_empty()).collect();
        if !deferrable.is_empty() {
            v.deferrable = Some(deferrable);
        }
        if let Some(sub) = refs(el, "submachine").into_iter().next() {
            v.submachine = Some(sub);
        }
        let vid = v.id.clone();
        let vi = self.model.vertices.len();
        self.model.vertices.push(v);
        for cp in kids(el, "connectionPoint") {
            self.read_point(cp, &vid);
        }
        for c in kids(el, "connection") {
            let entry_ref = refs(c, "entry").into_iter().next();
            let exit_ref = refs(c, "exit").into_iter().next();
            let mut r = Reader::vertex_base(c, VertexType::ConnectionPointRef, &vid);
            r.name = String::new();
            r.point_kind =
                Some(if exit_ref.is_some() && entry_ref.is_none() { PointKind::Exit } else { PointKind::Entry });
            r.point_ref = Some(local_id(&entry_ref.or(exit_ref).unwrap_or_default()));
            self.model.vertices.push(r);
        }
        for r in kids(el, "region") {
            self.model.vertices[vi].regions.push(Region {
                id: id_of(r),
                name: r.attr("name").unwrap_or("").to_string(),
                size: None,
            });
            self.read_region(r);
        }
    }

    fn read_region(&mut self, el: &XmlElement) {
        let id = id_of(el);
        for c in &el.children {
            match c.name.as_str() {
                "subvertex" => self.read_vertex(c, &id),
                "transition" => self.read_transition(c),
                "ownedComment" => {
                    let mut v = Reader::vertex_base(c, VertexType::Comment, &id);
                    v.text = Some(body_of(Some(c), false));
                    v.anchors = Some(refs(c, "annotatedElement"));
                    self.model.vertices.push(v);
                }
                _ => {}
            }
        }
    }
}

/// Parses a state machine file. The error is the message to show.
pub fn from_xmi(text: &str) -> Result<XmiModel, String> {
    let canonical: HashMap<&str, &str> =
        [(NS_XMI, "xmi"), (NS_UML, "uml"), (NS_UMLDI, "umldi"), (NS_DC, "dc"), (NS_DI, "di")].into_iter().collect();
    let root = parse_xml(text, &canonical)?;
    if root.name != "xmi:XMI" && root.name != "uml:Model" {
        return Err(format!("Not an XMI document: the root element is <{}>, expected <xmi:XMI>.", root.name));
    }
    let mut by_id: HashMap<String, &XmlElement> = HashMap::new();
    let mut machine: Option<&XmlElement> = None;
    let mut context: Option<&XmlElement> = None;
    fn visit<'a>(
        el: &'a XmlElement,
        parent: Option<&'a XmlElement>,
        by_id: &mut HashMap<String, &'a XmlElement>,
        machine: &mut Option<&'a XmlElement>,
        context: &mut Option<&'a XmlElement>,
    ) {
        let id = id_of(el);
        if !id.is_empty() {
            by_id.insert(id, el);
        }
        let t = type_of(el);
        if machine.is_none() && (t == "uml:StateMachine" || t == "uml:ProtocolStateMachine") {
            *machine = Some(el);
            if let Some(p) = parent
                && type_of(p) == "uml:Class"
            {
                *context = Some(p);
            }
        }
        for c in &el.children {
            visit(c, Some(el), by_id, machine, context);
        }
    }
    visit(&root, None, &mut by_id, &mut machine, &mut context);
    let Some(sm) = machine else { return Err("The document does not contain a UML state machine.".into()) };

    let xmi_id = Some(id_of(sm)).filter(|s| !s.is_empty()).unwrap_or_else(|| "sm".into());
    let mut model = FsmModel::new(sm.attr("name").unwrap_or(""));
    model.kind =
        if type_of(sm) == "uml:ProtocolStateMachine" { MachineKind::Protocol } else { MachineKind::Behavioral };
    model.documentation = Some(String::new());
    for c in kids(sm, "ownedComment") {
        if refs(c, "annotatedElement").contains(&xmi_id) {
            model.documentation = Some(body_of(Some(c), true));
        }
    }
    let mut r = Reader { by_id, model };
    for reg in kids(sm, "region") {
        r.model.regions.push(Region { id: id_of(reg), name: reg.attr("name").unwrap_or("").to_string(), size: None });
        r.read_region(reg);
    }
    if r.model.regions.is_empty() {
        r.model.regions.push(Region { id: format!("{xmi_id}_region"), name: String::new(), size: None });
    }
    let top = r.model.regions[0].id.clone();
    for cp in kids(sm, "connectionPoint") {
        r.read_point(cp, &top);
    }
    let mut model = r.model;

    // Stereotype applications: elements of the embedded profile's namespace (the first vertex of each id).
    let mut v_index: HashMap<String, usize> = HashMap::new();
    for (i, v) in model.vertices.iter().enumerate() {
        v_index.insert(v.id.clone(), i);
    }
    for c in &root.children {
        if c.ns != PROFILE_NS {
            continue;
        }
        if let Some(&i) = v_index.get(&local_id(c.attr("base_State").unwrap_or(""))) {
            model.vertices[i].stereotype = Some(c.local.clone());
        }
    }

    // Layout
    let mut t_index: HashMap<String, usize> = HashMap::new();
    for (i, t) in model.transitions.iter().enumerate() {
        t_index.insert(t.id.clone(), i);
    }
    let mut region_shapes: HashMap<String, (f64, f64, f64, f64)> = HashMap::new();
    for d in &root.children {
        if d.ns != NS_UMLDI {
            continue;
        }
        for ext in kids(d, "xmi:Extension") {
            if ext.attr("extender") != Some(EXTENDER) {
                continue;
            }
            for st in kids(ext, "style") {
                let element = st.attr("element").unwrap_or("");
                let color = st.attr("color").unwrap_or("").to_lowercase();
                let mut items = ItemColors::new();
                for k in ITEM_COLOR_KEYS {
                    let c = st.attr(if k == "action" { "effect" } else { k }).unwrap_or("").to_lowercase();
                    if is_color(&c) {
                        items.insert(k.to_string(), c);
                    }
                }
                let (c_slot, i_slot) = if let Some(&i) = v_index.get(element) {
                    let v = &mut model.vertices[i];
                    (&mut v.color, &mut v.item_colors)
                } else if let Some(&i) = t_index.get(element) {
                    let t = &mut model.transitions[i];
                    (&mut t.color, &mut t.item_colors)
                } else {
                    continue;
                };
                if is_color(&color) {
                    *c_slot = Some(color);
                }
                if !items.is_empty() {
                    *i_slot = Some(items);
                }
            }
            // A vertex listed in several groups stays in the first.
            for g in kids(ext, "group") {
                let id = g.attr("id").unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                for member in g.attr("members").unwrap_or("").split(js_space) {
                    if let Some(&i) = v_index.get(member)
                        && model.vertices[i].group.is_none()
                    {
                        model.vertices[i].group = Some(id.to_string());
                    }
                }
            }
        }
        let mut all = Vec::new();
        walk(d, &mut all);
        for el in all {
            let ty = type_of(el);
            let Some(target) =
                el.attr("modelElement").map(str::to_string).or_else(|| refs(el, "modelElement").into_iter().next())
            else {
                continue;
            };
            if target.is_empty() {
                continue;
            }
            let b = kid(el, "bounds");
            if ty == "umldi:UMLShape"
                && let Some(b) = b
            {
                let bx = (num0(b.attr("x")), num0(b.attr("y")), num0(b.attr("width")), num0(b.attr("height")));
                match v_index.get(&target) {
                    Some(&i) => {
                        let v = &mut model.vertices[i];
                        (v.x, v.y, v.w, v.h) = bx;
                    }
                    None => {
                        region_shapes.insert(target, bx);
                    }
                }
            } else if ty == "umldi:UMLEdge"
                && let Some(&ti) = t_index.get(&target)
            {
                let pts: Vec<Point> =
                    kids(el, "waypoint").map(|w| Point { x: num0(w.attr("x")), y: num0(w.attr("y")) }).collect();
                let t = &mut model.transitions[ti];
                if pts.len() > 2 {
                    t.points = Some(pts[1..pts.len() - 1].to_vec());
                }
                let label = el.children.iter().find(|c| type_of(c) == "umldi:UMLLabel");
                if let Some(lb) = label.and_then(|l| kid(l, "bounds"))
                    && pts.len() >= 2
                {
                    let mid = polyline_mid(&pts);
                    // Not rounded: the middle is off the 0.1 px grid, and an offset rounded to 0.1 px could move
                    // the label by 0.1 px on each save.
                    t.label_offset = Some(Point {
                        x: num(lb.attr("x")) + num(lb.attr("width")) / 2.0 - mid.x,
                        y: num(lb.attr("y")) + num(lb.attr("height")) / 2.0 - mid.y,
                    });
                }
            }
        }
    }
    for v in &mut model.vertices {
        if v.regions.len() >= 2
            && let (Some(a), Some(b)) = (region_shapes.get(&v.regions[0].id), region_shapes.get(&v.regions[1].id))
        {
            v.region_layout = Some(if (b.0 - a.0).abs() > (b.1 - a.1).abs() {
                RegionLayout::Horizontal
            } else {
                RegionLayout::Vertical
            });
        }
        // Each region's own size, along the layout.
        let horizontal = v.region_layout == Some(RegionLayout::Horizontal);
        for r in &mut v.regions {
            if let Some(bx) = region_shapes.get(&r.id) {
                r.size = Some(if horizontal { bx.2 } else { bx.3 });
            }
        }
    }

    // Elements without a shape get a default place and size.
    let mut slot = 0usize;
    for v in &mut model.vertices {
        if v.w == 0.0 || v.w.is_nan() || v.h == 0.0 || v.h.is_nan() {
            (v.w, v.h) = default_size(v.vtype);
        }
        if v.x.is_nan() || v.y.is_nan() {
            v.x = 40.0 + (slot % 5) as f64 * 180.0;
            v.y = 40.0 + (slot / 5) as f64 * 120.0;
            slot += 1;
        }
    }
    normalize_model(&mut model);
    Ok(XmiModel { model, xmi_id, context: context.and_then(|c| c.attr("name")).unwrap_or("").to_string() })
}
