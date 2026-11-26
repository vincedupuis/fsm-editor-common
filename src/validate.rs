//! Validation: fsm-cli's rules and messages, run on an editor's model (with its own ids).

use std::collections::HashMap;

use fsm_cli::model as cli;
use fsm_cli::validation::{self, Severity};
use serde::{Deserialize, Serialize};

use crate::model::{ConnectionPointInfo, FsmModel, MachineKind, PointKind, TransitionKind, VertexType, s};

/// What is known about a machine referenced by submachine states (by its href).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmachineInfo {
    #[serde(default)]
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub points: Vec<ConnectionPointInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    /// Id of the offending vertex, transition or region ('' for the machine itself).
    pub id: String,
    pub severity: &'static str,
    pub message: String,
}

fn vertex_type(t: VertexType) -> cli::VertexType {
    use cli::VertexType as C;
    match t {
        VertexType::State => C::State,
        VertexType::Final => C::Final,
        VertexType::Initial => C::Initial,
        VertexType::ShallowHistory => C::ShallowHistory,
        VertexType::DeepHistory => C::DeepHistory,
        VertexType::Choice => C::Choice,
        VertexType::Junction => C::Junction,
        VertexType::Fork => C::Fork,
        VertexType::Join => C::Join,
        VertexType::EntryPoint => C::EntryPoint,
        VertexType::ExitPoint => C::ExitPoint,
        VertexType::Terminate => C::Terminate,
        VertexType::ConnectionPointRef => C::ConnectionPointRef,
        VertexType::Comment => C::Comment,
    }
}

fn point_kind(k: PointKind) -> cli::PointKind {
    match k {
        PointKind::Entry => cli::PointKind::Entry,
        PointKind::Exit => cli::PointKind::Exit,
    }
}

/// The model as fsm-cli holds it (the same ids).
pub fn to_cli_model(m: &FsmModel) -> cli::FsmModel {
    cli::FsmModel {
        name: m.name.clone(),
        kind: if m.kind == MachineKind::Protocol { cli::MachineKind::Protocol } else { cli::MachineKind::Behavioral },
        context: s(&m.context).to_string(),
        documentation: s(&m.documentation).to_string(),
        regions: m.regions.iter().map(|r| cli::Region::new(&r.id, &r.name)).collect(),
        vertices: m
            .vertices
            .iter()
            .map(|v| {
                let mut c = cli::Vertex::new(&v.id, vertex_type(v.vtype), &v.name, &v.parent);
                c.regions = v.regions.iter().map(|r| cli::Region::new(&r.id, &r.name)).collect();
                c.entry = s(&v.entry).to_string();
                c.exit = s(&v.exit).to_string();
                c.do_activity = s(&v.do_activity).to_string();
                c.deferrable = v.deferrable.clone().unwrap_or_default();
                c.invariant = s(&v.invariant).to_string();
                c.submachine = v.submachine.clone();
                c.stereotype = s(&v.stereotype).to_string();
                c.point_ref = s(&v.point_ref).to_string();
                c.point_kind = point_kind(v.point_kind.unwrap_or(PointKind::Entry));
                c.text = s(&v.text).to_string();
                c.anchors = v.anchors.clone().unwrap_or_default();
                c
            })
            .collect(),
        transitions: m
            .transitions
            .iter()
            .map(|t| {
                let kind = match t.kind {
                    TransitionKind::External => cli::TransitionKind::External,
                    TransitionKind::Internal => cli::TransitionKind::Internal,
                    TransitionKind::Local => cli::TransitionKind::Local,
                };
                let mut c = cli::Transition::new(&t.id, &t.source, &t.target, kind);
                c.triggers = t.triggers.clone();
                c.guard = t.guard.clone();
                c.action = t.action.clone();
                c.precondition = s(&t.precondition).to_string();
                c.postcondition = s(&t.postcondition).to_string();
                c
            })
            .collect(),
    }
}

/// Checks the UML 2.5.1 well-formedness rules, names and references, the text grammar and reachability.
/// `submachines` describes the machines referenced by submachine states, by href, when known.
pub fn validate(model: &FsmModel, submachines: &HashMap<String, SubmachineInfo>) -> Vec<Issue> {
    let subs: HashMap<String, cli::SubmachineInfo> = submachines
        .iter()
        .map(|(href, i)| {
            (
                href.clone(),
                cli::SubmachineInfo {
                    found: i.found,
                    name: i.name.clone().unwrap_or_default(),
                    points: i
                        .points
                        .iter()
                        .map(|p| cli::ConnectionPointInfo {
                            id: p.id.clone(),
                            name: p.name.clone(),
                            kind: point_kind(p.kind),
                        })
                        .collect(),
                },
            )
        })
        .collect();
    validation::validate(&to_cli_model(model), &subs)
        .into_iter()
        .map(|i| Issue {
            id: i.id,
            severity: match i.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "info",
            },
            message: i.message,
        })
        .collect()
}
