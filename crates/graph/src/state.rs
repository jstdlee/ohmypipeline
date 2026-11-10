//! Per-node execution state. Pure bookkeeping over the graph; the worker
//! keeps the cached outputs, this crate keeps what the canvas should claim.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Graph, dependency_key};

/// What a node should show on its card.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeState {
    /// A required input is unwired, or a required param is empty.
    NotConfigured,
    /// Ready to run, nothing recorded yet.
    Configured,
    /// A job is in flight for this node.
    Running,
    /// The last run succeeded and the dependency key is unchanged.
    Executed,
    /// The last run failed with the message, and nothing changed since.
    Error(String),
    /// It ran or failed before, and the dependency key has changed since.
    Stale,
    /// The branch is skipped. Reserved for branches.
    Inactive,
}

/// Execution bookkeeping per node: the dependency key at the last success, the
/// last error with the key it happened at, and the running and inactive sets.
#[derive(Clone, Debug, Default)]
pub struct ExecState {
    keys: BTreeMap<String, u64>,
    errors: BTreeMap<String, (String, u64)>,
    ran: BTreeSet<String>,
    running: BTreeSet<String>,
    inactive: BTreeSet<String>,
}

impl ExecState {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when every required input has a wire and no required param is
    /// empty. Paths and code are required; so is a param port with no wire.
    pub fn configured(graph: &Graph, id: &str) -> bool {
        let Some(node) = graph.node(id) else {
            return false;
        };
        if node.unknown {
            return false;
        }
        let mut required: Vec<String> = match node.def() {
            Some(def) => def
                .inputs
                .iter()
                .map(|port| port.name.to_string())
                .collect(),
            None => Vec::new(),
        };
        if let Some(custom) = &node.inputs {
            for port in custom {
                if !required.iter().any(|name| name == &port.name) {
                    required.push(port.name.clone());
                }
            }
        }
        for port in &node.added_inputs {
            if !required.iter().any(|name| name == &port.name) {
                required.push(port.name.clone());
            }
        }
        let optional: &[&str] = node.def().map(|def| def.optional_inputs).unwrap_or(&[]);
        for port in &required {
            if optional.contains(&port.as_str()) {
                continue;
            }
            if graph.edges_into(id, port).next().is_none() {
                return false;
            }
        }
        // Wired params fall back to their literal when the wire is gone.
        for param in node.def().map(|def| def.params).unwrap_or(&[]) {
            let required_param =
                param.name == "path" || matches!(param.kind, crate::feature::ParamKind::Code);
            if !required_param {
                continue;
            }
            let wired = node.wired.contains(param.name);
            if wired && graph.edges_into(id, param.name).next().is_some() {
                continue;
            }
            if param_is_empty(node.params.get(param.name)) {
                return false;
            }
        }
        true
    }

    /// The state a card should show. Running and Inactive win; then an error
    /// whose key is unchanged; then success whose key is unchanged; then
    /// Stale for anything that ran or failed before; then the configured pair.
    pub fn state_of(&self, graph: &Graph, id: &str) -> NodeState {
        if self.running.contains(id) {
            return NodeState::Running;
        }
        if self.inactive.contains(id) {
            return NodeState::Inactive;
        }
        let key = dependency_key(graph, id);
        if let Some((message, error_key)) = self.errors.get(id)
            && *error_key == key
        {
            return NodeState::Error(message.clone());
        }
        if let Some(success_key) = self.keys.get(id)
            && *success_key == key
        {
            return NodeState::Executed;
        }
        if self.ran.contains(id) {
            return NodeState::Stale;
        }
        if !Self::configured(graph, id) {
            return NodeState::NotConfigured;
        }
        NodeState::Configured
    }

    /// A run started for these nodes.
    pub fn start(&mut self, ids: &[String]) {
        for id in ids {
            self.running.insert(id.clone());
        }
    }

    /// These nodes finished, or were served from the cache. Their current
    /// dependency keys become the keys of the last success.
    pub fn record_success(&mut self, graph: &Graph, computed_ids: &[String]) {
        for id in computed_ids {
            self.running.remove(id);
            self.ran.insert(id.clone());
            self.errors.remove(id);
            self.keys.insert(id.clone(), dependency_key(graph, id));
        }
    }

    /// The run failed at `id` with `message`, at its current dependency key.
    pub fn record_error(&mut self, graph: &Graph, id: &str, message: String) {
        self.running.remove(id);
        self.ran.insert(id.to_string());
        self.errors
            .insert(id.to_string(), (message, dependency_key(graph, id)));
    }

    /// Marks the active branch. Replaces the set.
    pub fn set_inactive(&mut self, ids: &[String]) {
        self.inactive = ids.iter().cloned().collect();
    }

    /// Forgets `id` and everything downstream of it.
    pub fn reset(&mut self, graph: &Graph, id: &str) {
        let mut gone = vec![id.to_string()];
        gone.extend(crate::model::descendants(graph, id));
        for id in gone {
            self.keys.remove(&id);
            self.errors.remove(&id);
            self.ran.remove(&id);
            self.running.remove(&id);
            self.inactive.remove(&id);
        }
    }
}

fn param_is_empty(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(text)) => text.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;
    use crate::model::Node;
    use serde_json::json;

    fn graph_with(nodes: Vec<Node>, edges: Vec<crate::model::Edge>) -> Graph {
        Graph {
            nodes,
            edges,
            ..Graph::default()
        }
    }

    fn chain() -> Graph {
        // n1 -> n2 -> n3, and n1 -> n4 as a sibling branch.
        graph_with(
            vec![
                Node::new("n1", "read_csv", Point::new(0.0, 0.0)),
                Node::new("n2", "head", Point::new(0.0, 0.0)),
                Node::new("n3", "head", Point::new(0.0, 0.0)),
                Node::new("n4", "head", Point::new(0.0, 0.0)),
            ],
            vec![
                crate::model::Edge::between("n1", "out", "n2", "in"),
                crate::model::Edge::between("n2", "out", "n3", "in"),
                crate::model::Edge::between("n1", "out", "n4", "in"),
            ],
        )
    }

    fn set_param(graph: &mut Graph, id: &str, name: &str, value: serde_json::Value) {
        graph
            .node_mut(id)
            .expect("node")
            .params
            .insert(name.to_string(), value);
    }

    #[test]
    fn unconfigured_when_a_required_input_or_param_is_missing() {
        let mut graph = Graph::default();
        graph
            .nodes
            .push(Node::new("n1", "head", Point::new(0.0, 0.0)));
        assert!(!ExecState::configured(&graph, "n1"));
        let mut graph = chain();
        assert!(
            !ExecState::configured(&graph, "n1"),
            "reader without a path"
        );
        set_param(&mut graph, "n1", "path", json!("orders.csv"));
        assert!(ExecState::configured(&graph, "n1"));
        assert!(ExecState::configured(&graph, "n2"));
        let mut bare = Graph::default();
        let mut python = Node::new("p", "python", Point::new(0.0, 0.0));
        python.inputs = None; // a code block with no ports, so only the code matters
        python.params.insert("code".into(), json!(""));
        bare.nodes.push(python);
        assert!(!ExecState::configured(&bare, "p"), "empty code");
        bare.node_mut("p")
            .unwrap()
            .params
            .insert("code".into(), json!("def run(df):\n    return df\n"));
        assert!(ExecState::configured(&bare, "p"));
    }

    #[test]
    fn an_edit_makes_the_branch_stale_and_the_sibling_stays_executed() {
        let mut graph = chain();
        set_param(&mut graph, "n1", "path", json!("orders.csv"));
        let mut state = ExecState::new();
        let all: Vec<String> = vec!["n1".into(), "n2".into(), "n3".into(), "n4".into()];
        state.record_success(&graph, &all);
        for id in &all {
            assert_eq!(state.state_of(&graph, id), NodeState::Executed, "{id}");
        }
        set_param(&mut graph, "n2", "n", json!(2));
        assert_eq!(state.state_of(&graph, "n1"), NodeState::Executed);
        assert_eq!(state.state_of(&graph, "n2"), NodeState::Stale);
        assert_eq!(state.state_of(&graph, "n3"), NodeState::Stale);
        assert_eq!(state.state_of(&graph, "n4"), NodeState::Executed);
    }

    #[test]
    fn running_and_inactive_win_over_everything() {
        let graph = chain();
        let mut state = ExecState::new();
        state.start(&["n2".to_string()]);
        assert_eq!(state.state_of(&graph, "n2"), NodeState::Running);
        state.set_inactive(&["n4".to_string()]);
        assert_eq!(state.state_of(&graph, "n4"), NodeState::Inactive);
    }

    #[test]
    fn an_error_holds_until_the_key_changes_then_it_is_stale_then_executed() {
        let mut graph = chain();
        set_param(&mut graph, "n1", "path", json!("orders.csv"));
        let mut state = ExecState::new();
        state.record_error(&graph, "n2", "boom".into());
        assert_eq!(
            state.state_of(&graph, "n2"),
            NodeState::Error("boom".into())
        );
        set_param(&mut graph, "n2", "n", json!(1));
        assert_eq!(state.state_of(&graph, "n2"), NodeState::Stale);
        state.record_success(&graph, &["n2".to_string()]);
        assert_eq!(state.state_of(&graph, "n2"), NodeState::Executed);
        // A node that never ran stays Configured, not Stale.
        assert_eq!(state.state_of(&graph, "n3"), NodeState::Configured);
    }

    #[test]
    fn reset_forgets_the_node_and_its_descendants_only() {
        let mut graph = chain();
        set_param(&mut graph, "n1", "path", json!("orders.csv"));
        let mut state = ExecState::new();
        let all: Vec<String> = vec!["n1".into(), "n2".into(), "n3".into(), "n4".into()];
        state.record_success(&graph, &all);
        state.reset(&graph, "n2");
        assert_eq!(state.state_of(&graph, "n2"), NodeState::Configured);
        assert_eq!(state.state_of(&graph, "n3"), NodeState::Configured);
        assert_eq!(state.state_of(&graph, "n1"), NodeState::Executed);
        assert_eq!(state.state_of(&graph, "n4"), NodeState::Executed);
    }
}
