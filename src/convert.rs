//! Converting files of earlier versions (XMI, xmi.rs) to the JSON format (format.rs). Machines linked by submachine
//! references are converted together: a connection point reference names its point by xmi:id in the other file, and
//! becomes that point's name, which the conversion of the other file may give or change.
//!
//! Names: unnamed elements get generated names (`choice1*`, hidden on the diagram), `*` and `.` in names become `_`,
//! a name used twice gets a number (`Idle2`). What the JSON format can't hold is left out and reported.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::format::{assign_names, default_size, resizable, write_machine};
use crate::model::{FsmModel, TransitionKind, VertexType, is_color};
use crate::paths::{decode_href, dirname, join_path};
use crate::xmi::{XmiModel, from_xmi};

/// An old file to convert: its path (`/`-separated, absolute) and text.
#[derive(Debug, Clone, Deserialize)]
pub struct ConvertInput {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Converted {
    pub path: String,
    /// The converted model (its ids are the old file's); None when the file could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<FsmModel>,
    /// The texts of the two new files.
    pub model_text: String,
    pub layout_text: String,
    /// Names changed, as sentences.
    pub renames: Vec<String>,
    /// How many unnamed elements got a generated name.
    pub generated: usize,
    /// What was left out, as sentences.
    pub losses: Vec<String>,
    /// Why the file could not be read (nothing else is set then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn label(t: VertexType) -> &'static str {
    match t {
        VertexType::Initial => "initial pseudostate",
        VertexType::ShallowHistory => "shallow history",
        VertexType::DeepHistory => "deep history",
        VertexType::Choice => "choice",
        VertexType::Junction => "junction",
        VertexType::Final => "final state",
        VertexType::Terminate => "terminate pseudostate",
        VertexType::EntryPoint => "entry point",
        VertexType::ExitPoint => "exit point",
        other => other.key(),
    }
}

/// Converts old files together (a machine and the machines linked to it by submachine references).
pub fn convert_group(inputs: &[ConvertInput]) -> Vec<Converted> {
    let mut read: Vec<(String, XmiModel)> = Vec::new();
    let mut results: Vec<Converted> = Vec::new();
    for i in inputs {
        match from_xmi(&i.text) {
            Ok(m) => match read.iter_mut().find(|(p, _)| *p == i.path) {
                Some(slot) => slot.1 = m,
                None => read.push((i.path.clone(), m)),
            },
            Err(e) => results.push(Converted {
                path: i.path.clone(),
                model: None,
                model_text: String::new(),
                layout_text: String::new(),
                renames: Vec::new(),
                generated: 0,
                losses: Vec::new(),
                error: Some(e),
            }),
        }
    }
    // Names first, in every file: the points' new names are what the references become.
    let mut named: HashMap<String, (Vec<String>, usize, Vec<String>)> = HashMap::new();
    for (path, m) in read.iter_mut() {
        let losses = old_only(m);
        let unnamed = count_unnamed(&m.model);
        let mut renames: Vec<String> = Vec::new();
        // The context becomes a name, as the element names do.
        let context = crate::format::clean_name(&m.context);
        if context != m.context {
            renames.push(format!("the context '{}' of the state machine is now '{context}'", m.context));
        }
        m.model.context = Some(context);
        renames.extend(assign_names(&mut m.model, true).into_iter().map(|r| {
            format!("{} '{}' is now '{}'{}", r.what, r.from, r.to, r.why.map(|w| format!(" ({w})")).unwrap_or_default())
        }));
        // UML makes a transition leaving an entry point local: earlier versions wrote them external.
        let points: Vec<(String, String)> = m
            .model
            .vertices
            .iter()
            .filter(|v| v.vtype == VertexType::EntryPoint)
            .map(|v| (v.id.clone(), v.name.clone()))
            .collect();
        for (id, name) in points {
            let mut changed = false;
            for t in m.model.transitions.iter_mut().filter(|t| t.source == id && t.kind == TransitionKind::External) {
                t.kind = TransitionKind::Local;
                changed = true;
            }
            if changed {
                renames.push(format!("the transitions leaving entry point '{name}' are now local"));
            }
        }
        named.insert(path.clone(), (renames, unnamed, losses));
    }
    for k in 0..read.len() {
        let path = read[k].0.clone();
        let mut extra = Vec::new();
        let n = read[k].1.model.vertices.len();
        for vi in 0..n {
            let v = &read[k].1.model.vertices[vi];
            let Some(sub) = v.submachine.clone().filter(|s| v.vtype == VertexType::State && !s.is_empty()) else {
                continue;
            };
            let (vid, vname) = (v.id.clone(), v.name.clone());
            let (file, id) = {
                let mut parts = sub.split('#');
                (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string())
            };
            let sub_path = join_path(&dirname(&path), &decode_href(&file));
            let found: Option<&XmiModel> =
                read.iter().find(|(p, _)| *p == sub_path).map(|(_, m)| m).filter(|m| id.is_empty() || m.xmi_id == id);
            let names: Vec<(String, String)> = found
                .map(|f| {
                    f.model
                        .vertices
                        .iter()
                        .filter(|p| matches!(p.vtype, VertexType::EntryPoint | VertexType::ExitPoint))
                        .map(|p| (p.id.clone(), p.name.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let m = &mut read[k].1.model;
            m.vertices[vi].submachine = Some(decode_href(&file));
            for r in m.vertices.iter_mut() {
                if r.vtype != VertexType::ConnectionPointRef || r.parent != vid {
                    continue;
                }
                let rref = r.point_ref.clone().unwrap_or_default();
                match names.iter().find(|(id, _)| *id == rref) {
                    Some((_, name)) => r.point_ref = Some(name.clone()),
                    None => {
                        if !rref.is_empty() {
                            extra.push(format!(
                                "the connection point reference of state '{vname}' to point '{rref}' of {file}, which was not found: it keeps that id as the point's name"
                            ));
                        }
                    }
                }
            }
        }
        named.get_mut(&path).unwrap().2.extend(extra);
    }
    for (path, m) in &read {
        let (renames, generated, losses) = named.remove(path).unwrap();
        let w = write_machine(&m.model);
        let mut all = losses;
        all.extend(w.lost);
        results.push(Converted {
            path: path.clone(),
            model: Some(m.model.clone()),
            model_text: w.model,
            layout_text: w.layout,
            renames,
            generated,
            losses: all,
            error: None,
        });
    }
    inputs.iter().map(|i| results.iter().find(|r| r.path == i.path).unwrap().clone()).collect()
}

/// Elements that need a name and have none.
fn count_unnamed(m: &FsmModel) -> usize {
    let named = |t: VertexType| {
        matches!(
            t,
            VertexType::State
                | VertexType::Choice
                | VertexType::Junction
                | VertexType::Fork
                | VertexType::Join
                | VertexType::Final
                | VertexType::Terminate
                | VertexType::Comment
                | VertexType::EntryPoint
                | VertexType::ExitPoint
        )
    };
    let mut n = m.vertices.iter().filter(|v| named(v.vtype) && v.name.is_empty()).count();
    let all = std::iter::once(&m.regions).chain(m.vertices.iter().map(|v| &v.regions));
    for regions in all {
        if regions.len() > 1 {
            n += regions.iter().filter(|r| r.name.is_empty()).count();
        }
    }
    n
}

fn js_num(n: f64) -> String {
    if n.is_finite() {
        fsm_cli::json::number(n)
    } else if n.is_nan() {
        "NaN".into()
    } else if n > 0.0 {
        "Infinity".into()
    } else {
        "-Infinity".into()
    }
}

/// What an old file holds that the JSON format has no place for, besides what the writer reports.
fn old_only(x: &mut XmiModel) -> Vec<String> {
    let mut out = Vec::new();
    let m = &mut x.model;
    for v in m.vertices.iter_mut() {
        let describe = format!(
            "the {}{}",
            label(v.vtype),
            if v.name.is_empty() { String::new() } else { format!(" '{}'", v.name) }
        );
        let pseudo = matches!(v.vtype, VertexType::Initial | VertexType::ShallowHistory | VertexType::DeepHistory);
        if pseudo && !v.name.is_empty() {
            out.push(format!("the name of {describe}"));
        }
        if pseudo && (v.color.as_deref().is_some_and(is_color) || v.item_colors.is_some()) {
            out.push(format!("the color of {describe}"));
        }
        if !resizable(v.vtype) && v.vtype != VertexType::Comment {
            let (w, h) = default_size(v.vtype);
            if (v.w - w).abs() > 0.05 || (v.h - h).abs() > 0.05 {
                out.push(format!(
                    "the size of {describe} ({} × {}): it is drawn at its default size, around the same center",
                    js_num(v.w),
                    js_num(v.h)
                ));
                v.x += (v.w - w) / 2.0;
                v.y += (v.h - h) / 2.0;
                (v.w, v.h) = (w, h);
            }
        }
    }
    out
}
