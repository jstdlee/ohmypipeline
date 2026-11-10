use crate::feature::feature;
use crate::geom::SNAP_RADIUS_PX;
use crate::model::{Graph, Node, literal_type};
use crate::types::TypeName;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Type,
    Full,
    Duplicate,
    Cycle,
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Type => "type",
            Self::Full => "full",
            Self::Duplicate => "duplicate",
            Self::Cycle => "cycle",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkEnd {
    pub node: String,
    pub port: String,
}

#[derive(Clone, Debug)]
pub struct SnapCandidate {
    pub node: String,
    pub port: String,
    /// Window pixels, measured after the camera.
    pub distance_px: f32,
    pub compatible: bool,
}

/// Nearest compatible port inside the snap radius.
/// An incompatible port inside the radius does not win, even when it is closer.
pub fn pick_snap(candidates: &[SnapCandidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.compatible && candidate.distance_px <= SNAP_RADIUS_PX)
        .min_by(|(_, a), (_, b)| {
            a.distance_px
                .partial_cmp(&b.distance_px)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

pub fn reject_message(reason: Reject, from_type: &str, to_types: &str) -> String {
    match reason {
        Reject::Type => format!("{from_type} cannot connect to {to_types}"),
        Reject::Full => "that input already has a wire".to_string(),
        Reject::Duplicate => "those ports are already connected".to_string(),
        Reject::Cycle => "that connection would cycle".to_string(),
    }
}

fn preserves_input(feature: &str) -> bool {
    matches!(
        feature,
        "head"
            | "tail"
            | "sample"
            | "sort_values"
            | "concat"
            | "dropna"
            | "fillna"
            | "replace"
            | "drop_duplicates"
            | "reset_index"
            | "abs"
            | "round"
            | "shift"
            | "ffill"
            | "bfill"
            | "interpolate"
            | "clip"
            | "where"
            | "isin"
            | "between"
            | "loc"
            | "iloc"
            | "rolling"
            | "diff"
            | "pct_change"
            | "cumsum"
            | "str"
            | "dt"
            | "to_datetime"
            | "to_numeric"
            | "expanding"
            | "ewm"
            | "tz_localize"
            | "tz_convert"
            | "to_timedelta"
            | "cut"
            | "qcut"
            | "factorize"
            | "arith"
            | "rank"
            | "reindex"
            | "sort_index"
            | "map"
    )
}

fn feature_outputs(node: &Node) -> Vec<TypeName> {
    feature(&node.feature)
        .map(|def| {
            // A multi-output feature lists the types per named output.
            if !def.outputs.is_empty() {
                let mut types: Vec<TypeName> = Vec::new();
                for port in def.outputs {
                    for ty in port.accepts {
                        if !types.contains(ty) {
                            types.push(*ty);
                        }
                    }
                }
                return types;
            }
            if let Some(output) = def.output {
                vec![output]
            } else {
                def.possible_outputs.to_vec()
            }
        })
        .unwrap_or_default()
}

/// Types this node's output can be right now.
///
/// A known upstream collapses `head` and `sort` to that type. With no upstream,
/// every type the feature declares stays possible, so a wire is allowed when
/// any of them fits. Once the upstream is known, a mismatch is a type error.
pub fn output_types(graph: &Graph, node_id: &str) -> Vec<TypeName> {
    let Some(node) = graph.node(node_id) else {
        return Vec::new();
    };
    if node.unknown {
        return Vec::new();
    }
    match node.feature.as_str() {
        "chart" => Vec::new(),
        "literal" => vec![literal_type(node)],
        "python" => node
            .output
            .as_ref()
            .map(|port| port.accepts.clone())
            .unwrap_or_else(|| vec![TypeName::Dataframe]),
        "str" if node.param_str("method") == "extract" => vec![TypeName::Dataframe],
        feature if preserves_input(feature) => {
            if let Some(concrete) = same_as_input(graph, node) {
                vec![concrete]
            } else {
                feature_outputs(node)
            }
        }
        _ => feature_outputs(node),
    }
}

fn same_as_input(graph: &Graph, node: &Node) -> Option<TypeName> {
    let ports = node.input_ports();
    let port = ports.first()?;
    let incoming: Vec<_> = graph.edges_into(&node.id, &port.name).collect();
    if incoming.len() != 1 {
        return None;
    }
    let parent = &incoming[0].from;
    let types = output_types(graph, parent);
    if types.len() == 1 {
        Some(types[0])
    } else {
        None
    }
}

/// The single type a wire believes, when the node has one.
pub fn produced_type(graph: &Graph, node_id: &str) -> Option<TypeName> {
    let types = output_types(graph, node_id);
    if types.len() == 1 {
        Some(types[0])
    } else {
        None
    }
}

pub fn would_cycle(graph: &Graph, from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    reaches(graph, to, from)
}

fn reaches(graph: &Graph, start: &str, goal: &str) -> bool {
    let mut stack = vec![start.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if id == goal {
            return true;
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        for edge in graph.edges.iter().filter(|edge| edge.from == id) {
            stack.push(edge.to.clone());
        }
    }
    false
}

pub fn try_connect(
    graph: &Graph,
    from: &str,
    from_port: &str,
    to: &str,
    to_port: &str,
) -> Result<(), Reject> {
    if graph.node(from).is_none() || graph.node(to).is_none() {
        return Err(Reject::Type);
    }
    let source = graph.node(from).expect("checked");
    let target = graph.node(to).expect("checked");
    if source.unknown || target.unknown {
        return Err(Reject::Type);
    }
    // A multi-output feature connects from one named output port.
    if source
        .output_ports()
        .into_iter()
        .all(|port| port.name != from_port)
    {
        return Err(Reject::Type);
    }
    let Some(input) = target
        .input_ports()
        .into_iter()
        .find(|port| port.name == to_port)
    else {
        return Err(Reject::Type);
    };
    let produced = output_types(graph, from);
    if produced.is_empty() || !produced.iter().any(|ty| input.accepts.contains(ty)) {
        return Err(Reject::Type);
    }
    if graph.edges.iter().any(|edge| {
        edge.from == from && edge.from_port == from_port && edge.to == to && edge.to_port == to_port
    }) {
        return Err(Reject::Duplicate);
    }
    if !input.multi && graph.edges_into(to, to_port).next().is_some() {
        return Err(Reject::Full);
    }
    if would_cycle(graph, from, to) {
        return Err(Reject::Cycle);
    }
    Ok(())
}

/// A problem with an edge that is already on the graph.
/// Moving a card does not create one. A later, more specific upstream type can.
pub fn edge_problem(graph: &Graph, edge: &crate::model::Edge) -> Option<Reject> {
    // Duplicate and full are about adding. An edge that is already stored
    // is rechecked for type and cycle only.
    let source = graph.node(&edge.from)?;
    let target = graph.node(&edge.to)?;
    if source.unknown || target.unknown {
        return Some(Reject::Type);
    }
    let input = target
        .input_ports()
        .into_iter()
        .find(|port| port.name == edge.to_port)?;
    let produced = output_types(graph, &edge.from);
    if produced.is_empty() || !produced.iter().any(|ty| input.accepts.contains(ty)) {
        return Some(Reject::Type);
    }
    if would_cycle_existing(graph, edge) {
        return Some(Reject::Cycle);
    }
    None
}

fn would_cycle_existing(graph: &Graph, edge: &crate::model::Edge) -> bool {
    // Ignore this edge, then ask if `to` already reaches `from`.
    let mut stack = vec![edge.to.clone()];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if id == edge.from {
            return true;
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        for next in graph
            .edges
            .iter()
            .filter(|other| other.from == id && other.id != edge.id)
        {
            stack.push(next.to.clone());
        }
    }
    false
}

pub fn type_pair_label(graph: &Graph, from: &str, to: &str, to_port: &str) -> (String, String) {
    let from_label = output_types(graph, from)
        .into_iter()
        .map(|ty| ty.to_string())
        .collect::<Vec<_>>()
        .join("|");
    let to_label = graph
        .node(to)
        .and_then(|node| {
            node.input_ports()
                .into_iter()
                .find(|port| port.name == to_port)
        })
        .map(|port| {
            port.accepts
                .iter()
                .map(|ty| ty.to_string())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    (
        if from_label.is_empty() {
            "nothing".to_string()
        } else {
            from_label
        },
        if to_label.is_empty() {
            "nothing".to_string()
        } else {
            to_label
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;
    use crate::model::Node;
    use crate::types::TypeName;

    fn node(id: &str, feature: &str) -> Node {
        Node::new(id, feature, Point::new(0.0, 0.0))
    }

    #[test]
    fn series_into_change_headers_is_rejected() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "column"));
        graph.nodes.push(node("n2", "change_headers"));
        let err = try_connect(&graph, "n1", "out", "n2", "in").unwrap_err();
        assert_eq!(err, Reject::Type);
    }

    #[test]
    fn column_output_is_accepted_by_a_series_port() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "column"));
        graph.nodes.push(node("n2", "to_frame"));
        assert!(try_connect(&graph, "n1", "out", "n2", "in").is_ok());
        assert_eq!(produced_type(&graph, "n1"), Some(TypeName::Series));
    }

    #[test]
    fn a_second_wire_into_a_single_input_is_full() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "read_csv"));
        graph.nodes.push(node("n2", "read_csv"));
        graph.nodes.push(node("n3", "head"));
        graph
            .edges
            .push(crate::model::Edge::between("n1", "out", "n3", "in"));
        assert_eq!(
            try_connect(&graph, "n2", "out", "n3", "in").unwrap_err(),
            Reject::Full
        );
    }

    #[test]
    fn the_same_ports_cannot_be_linked_twice() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "read_csv"));
        graph.nodes.push(node("n2", "head"));
        graph
            .edges
            .push(crate::model::Edge::between("n1", "out", "n2", "in"));
        assert_eq!(
            try_connect(&graph, "n1", "out", "n2", "in").unwrap_err(),
            Reject::Duplicate
        );
    }

    #[test]
    fn a_cycle_is_rejected() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "head"));
        graph.nodes.push(node("n2", "head"));
        graph
            .edges
            .push(crate::model::Edge::between("n1", "out", "n2", "in"));
        assert_eq!(
            try_connect(&graph, "n2", "out", "n1", "in").unwrap_err(),
            Reject::Cycle
        );
        assert!(would_cycle(&graph, "n1", "n1"));
    }

    #[test]
    fn concat_accepts_a_second_wire_on_its_multi_port() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "read_csv"));
        graph.nodes.push(node("n2", "read_csv"));
        graph.nodes.push(node("n3", "concat"));
        graph
            .edges
            .push(crate::model::Edge::between("n1", "out", "n3", "in"));
        assert!(try_connect(&graph, "n2", "out", "n3", "in").is_ok());
    }

    #[test]
    fn one_output_may_feed_two_inputs() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "read_csv"));
        graph.nodes.push(node("n2", "head"));
        graph.nodes.push(node("n3", "query"));
        graph
            .edges
            .push(crate::model::Edge::between("n1", "out", "n2", "in"));
        assert!(try_connect(&graph, "n1", "out", "n3", "in").is_ok());
    }

    #[test]
    fn snap_picks_the_nearer_compatible_port() {
        let candidates = vec![
            SnapCandidate {
                node: "near-bad".into(),
                port: "in".into(),
                distance_px: 4.0,
                compatible: false,
            },
            SnapCandidate {
                node: "far-ok".into(),
                port: "in".into(),
                distance_px: 11.0,
                compatible: true,
            },
            SnapCandidate {
                node: "closer-ok".into(),
                port: "left".into(),
                distance_px: 9.0,
                compatible: true,
            },
            SnapCandidate {
                node: "outside".into(),
                port: "in".into(),
                distance_px: 12.1,
                compatible: true,
            },
        ];
        let index = pick_snap(&candidates).unwrap();
        assert_eq!(candidates[index].node, "closer-ok");
    }

    #[test]
    fn pivot_accepts_a_table_and_rejects_a_column() {
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "read_csv"));
        graph.nodes.push(node("n2", "pivot"));
        graph.nodes.push(node("n3", "column"));
        assert!(try_connect(&graph, "n1", "out", "n2", "in").is_ok());
        assert_eq!(produced_type(&graph, "n2"), Some(TypeName::Dataframe));
        assert_eq!(
            try_connect(&graph, "n3", "out", "n2", "in").unwrap_err(),
            Reject::Type
        );
    }

    #[test]
    fn a_writer_is_a_sink_and_text_follows_its_input() {
        let writer = node("w", "to_csv");
        assert!(writer.output_port().is_none());
        let mut graph = Graph::default();
        graph.nodes.push(node("n1", "column"));
        graph.nodes.push(node("n2", "str"));
        assert!(try_connect(&graph, "n1", "out", "n2", "in").is_ok());
        graph
            .edges
            .push(crate::model::Edge::between("n1", "out", "n2", "in"));
        assert_eq!(produced_type(&graph, "n2"), Some(TypeName::Series));
        let mut wide = Graph::default();
        wide.nodes.push(node("n1", "read_csv"));
        wide.nodes.push(node("n2", "loc"));
        assert!(try_connect(&wide, "n1", "out", "n2", "in").is_ok());
    }

    #[test]
    fn preview_table_and_plot_are_sinks() {
        let table = node("t", "preview_table");
        let plot = node("p", "chart");
        assert!(table.output_port().is_none());
        assert!(plot.output_port().is_none());
        assert!(
            table
                .input_ports()
                .iter()
                .any(|port| port.accepts.contains(&TypeName::Dataframe))
        );
    }

    #[test]
    fn an_incompatible_port_does_not_snap_when_nothing_compatible_is_in_range() {
        let candidates = vec![SnapCandidate {
            node: "bad".into(),
            port: "in".into(),
            distance_px: 2.0,
            compatible: false,
        }];
        assert!(pick_snap(&candidates).is_none());
    }

    #[test]
    fn the_flow_features_have_their_ports() {
        let switch = node("s", "if_switch");
        let names: Vec<_> = switch
            .output_ports()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(names, vec!["true".to_string(), "false".to_string()]);
        // Single-output features keep working through output_port.
        let head = node("h", "head");
        assert_eq!(head.output_port().unwrap().name, "out");
        assert_eq!(head.output_ports().len(), 1);
        let end = node("e", "end_if");
        let inputs: Vec<_> = end.input_ports().iter().map(|p| p.name.clone()).collect();
        assert_eq!(inputs, vec!["true".to_string(), "false".to_string()]);
        let start = node("l", "loop_start");
        assert_eq!(start.output_port().unwrap().name, "out");
    }

    #[test]
    fn wires_may_come_from_the_true_and_false_outputs() {
        let mut graph = Graph::default();
        graph.nodes.push(node("s", "if_switch"));
        graph.nodes.push(node("t", "head"));
        graph.nodes.push(node("f", "head"));
        graph.nodes.push(node("e", "end_if"));
        assert!(try_connect(&graph, "s", "true", "t", "in").is_ok());
        assert!(try_connect(&graph, "s", "false", "f", "in").is_ok());
        assert!(try_connect(&graph, "t", "out", "e", "true").is_ok());
        assert!(try_connect(&graph, "f", "out", "e", "false").is_ok());
        // The old single-output name is refused on a multi-output node.
        assert_eq!(
            try_connect(&graph, "s", "out", "t", "in").unwrap_err(),
            Reject::Type
        );
        // And the produced type still fits a dataframe input.
        assert_eq!(produced_type(&graph, "s"), Some(TypeName::Dataframe));
    }

    #[test]
    fn an_if_switch_inside_a_loop_wires_like_a_table_step() {
        let mut graph = Graph::default();
        graph.nodes.push(node("l", "loop_start"));
        graph.nodes.push(node("r", "read_csv"));
        graph.nodes.push(node("s", "if_switch"));
        graph.nodes.push(node("h", "head"));
        graph.nodes.push(node("e", "loop_end"));
        assert!(try_connect(&graph, "l", "out", "s", "in").is_ok());
        assert!(try_connect(&graph, "s", "true", "h", "in").is_ok());
        assert!(try_connect(&graph, "h", "out", "e", "in").is_ok());
    }
}
