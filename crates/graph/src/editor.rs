use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::connect::{Reject, try_connect};
use crate::feature::{self, FeatureDef};
use crate::file::{self, FileError};
use crate::geom::{Point, snap_grid};
use crate::model::{Edge, Graph, Mark, MarkKind, Node, PortRecord};
use crate::types::TypeName;

#[derive(Clone, Debug)]
struct Snapshot {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    marks: Vec<Mark>,
}

impl Snapshot {
    fn capture(graph: &Graph) -> Self {
        Self {
            nodes: graph.nodes.clone(),
            edges: graph.edges.clone(),
            marks: graph.marks.clone(),
        }
    }

    fn same(&self, graph: &Graph) -> bool {
        self.nodes == graph.nodes && self.edges == graph.edges && self.marks == graph.marks
    }

    fn restore(&self, graph: &mut Graph) {
        graph.nodes = self.nodes.clone();
        graph.edges = self.edges.clone();
        graph.marks = self.marks.clone();
    }
}

/// Undo stores the records a gesture replaced. It does not store pixels.
#[derive(Clone, Debug)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The graph as it was when the open gesture started.
    open: Option<Snapshot>,
    limit: usize,
}

impl History {
    fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
            limit: 200,
        }
    }

    fn begin(&mut self, graph: &Graph) {
        if self.open.is_none() {
            self.open = Some(Snapshot::capture(graph));
        }
    }

    fn commit(&mut self, graph: &Graph) {
        let Some(base) = self.open.take() else {
            return;
        };
        if base.same(graph) {
            return;
        }
        self.undo.push(base);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn edit(&mut self, graph: &mut Graph, body: impl FnOnce(&mut Graph)) {
        let opened = self.open.is_none();
        if opened {
            self.begin(graph);
        }
        body(graph);
        if opened {
            self.commit(graph);
        }
    }

    fn undo(&mut self, graph: &mut Graph) -> bool {
        if self.open.is_some() {
            return false;
        }
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot::capture(graph));
        prev.restore(graph);
        true
    }

    fn redo(&mut self, graph: &mut Graph) -> bool {
        if self.open.is_some() {
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot::capture(graph));
        next.restore(graph);
        true
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    graph: Graph,
    history: History,
    path: Option<PathBuf>,
    dirty: bool,
    readonly: bool,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            graph: Graph::default(),
            history: History::new(),
            path: None,
            dirty: false,
            readonly: false,
        }
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty() && self.history.open.is_none()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty() && self.history.open.is_none()
    }

    pub fn set_camera(&mut self, camera: crate::geom::Camera) {
        self.graph.camera = camera;
        self.dirty = true;
    }

    pub fn load(path: &Path) -> Result<Self, FileError> {
        let graph = file::load_path(path)?;
        Ok(Self {
            graph,
            history: History::new(),
            path: Some(path.to_path_buf()),
            dirty: false,
            readonly: !can_write(path),
        })
    }

    pub fn from_graph(graph: Graph) -> Self {
        Self {
            graph,
            history: History::new(),
            path: None,
            dirty: false,
            readonly: false,
        }
    }

    pub fn save(&mut self) -> Result<(), FileError> {
        let Some(path) = self.path.clone() else {
            return Err(FileError::Format("no path".into()));
        };
        if self.readonly {
            return Err(FileError::Format(format!(
                "{} is read-only",
                path.display()
            )));
        }
        file::save_atomic(&path, &self.graph)?;
        let recovery = file::recovery_path(&path);
        if recovery.exists() {
            let _ = std::fs::remove_file(recovery);
        }
        self.dirty = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), FileError> {
        file::save_atomic(path, &self.graph)?;
        self.path = Some(path.to_path_buf());
        self.readonly = !can_write(path);
        self.dirty = false;
        let recovery = file::recovery_path(path);
        if recovery.exists() {
            let _ = std::fs::remove_file(recovery);
        }
        Ok(())
    }

    pub fn write_recovery(&self) -> Result<(), FileError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !self.dirty {
            return Ok(());
        }
        file::save_atomic(&file::recovery_path(path), &self.graph)
    }

    /// Start a gesture. Further edits join it until [`commit_gesture`] or [`cancel_gesture`].
    pub fn begin_gesture(&mut self) {
        self.history.begin(&self.graph);
    }

    pub fn commit_gesture(&mut self) {
        let before = self.history.undo.len();
        self.history.commit(&self.graph);
        if self.history.undo.len() != before {
            self.dirty = true;
        }
    }

    /// Restore the records from the start of the gesture and write no undo entry.
    pub fn cancel_gesture(&mut self) {
        if let Some(base) = self.history.open.take() {
            base.restore(&mut self.graph);
        }
    }

    pub fn undo(&mut self) -> bool {
        if self.readonly {
            return false;
        }
        let changed = self.history.undo(&mut self.graph);
        if changed {
            self.dirty = true;
        }
        changed
    }

    pub fn redo(&mut self) -> bool {
        if self.readonly {
            return false;
        }
        let changed = self.history.redo(&mut self.graph);
        if changed {
            self.dirty = true;
        }
        changed
    }

    fn mutate(&mut self, body: impl FnOnce(&mut Graph)) -> bool {
        if self.readonly {
            return false;
        }
        self.history.edit(&mut self.graph, body);
        self.dirty = true;
        true
    }

    pub fn add_node(&mut self, feature_id: &str, position: Point) -> Option<String> {
        if feature::feature(feature_id).is_none() || self.readonly {
            return None;
        }
        let id = self.graph.alloc_node_id();
        let position = snap_grid(position);
        self.mutate(|graph| {
            graph
                .nodes
                .push(Node::new(id.clone(), feature_id, position));
        });
        Some(id)
    }

    pub fn add_after(&mut self, selected: &str, feature_id: &str) -> Option<String> {
        let source = self.graph.node(selected)?.clone();
        let position = snap_grid(Point::new(
            source.position[0] + crate::card_width(&source.feature) + 48.0,
            source.position[1],
        ));
        let id = self.add_node(feature_id, position)?;
        let new_node = self.graph.node(&id)?.clone();
        if let (Some(output), Some(input)) = (
            source.output_port(),
            new_node.input_ports().into_iter().next(),
        ) {
            let _ = self.connect(&source.id, &output.name, &id, &input.name);
        }
        Some(id)
    }

    pub fn move_nodes(&mut self, ids: &[String], delta: Point, snap: bool) {
        if self.readonly || ids.is_empty() {
            return;
        }
        let open = self.history.open.is_some();
        let apply = |graph: &mut Graph| {
            for node in &mut graph.nodes {
                if ids.iter().any(|id| id == &node.id) {
                    let mut point =
                        Point::new(node.position[0] + delta.x, node.position[1] + delta.y);
                    if snap {
                        point = snap_grid(point);
                    }
                    node.position = [point.x, point.y];
                }
            }
            for mark in &mut graph.marks {
                if ids.iter().any(|id| id == &mark.id) {
                    let mut point =
                        Point::new(mark.position[0] + delta.x, mark.position[1] + delta.y);
                    if snap {
                        point = snap_grid(point);
                    }
                    mark.position = [point.x, point.y];
                }
            }
        };
        if open {
            apply(&mut self.graph);
        } else {
            self.mutate(apply);
        }
    }

    /// Place nodes at absolute canvas positions. Used while a drag is open.
    pub fn place(
        &mut self,
        nodes: &BTreeMap<String, Point>,
        marks: &BTreeMap<String, Point>,
        snap: bool,
    ) {
        if self.readonly {
            return;
        }
        let apply = |graph: &mut Graph| {
            for node in &mut graph.nodes {
                if let Some(point) = nodes.get(&node.id) {
                    let point = if snap { snap_grid(*point) } else { *point };
                    node.position = [point.x, point.y];
                }
            }
            for mark in &mut graph.marks {
                if let Some(point) = marks.get(&mark.id) {
                    let point = if snap { snap_grid(*point) } else { *point };
                    mark.position = [point.x, point.y];
                }
            }
        };
        if self.history.open.is_some() {
            apply(&mut self.graph);
        } else {
            self.mutate(apply);
        }
    }

    pub fn delete(&mut self, node_ids: &[String], mark_ids: &[String]) {
        self.mutate(|graph| {
            let node_set: BTreeSet<_> = node_ids.iter().cloned().collect();
            let mark_set: BTreeSet<_> = mark_ids.iter().cloned().collect();
            graph.nodes.retain(|node| !node_set.contains(&node.id));
            graph.marks.retain(|mark| !mark_set.contains(&mark.id));
            graph
                .edges
                .retain(|edge| !node_set.contains(&edge.from) && !node_set.contains(&edge.to));
        });
    }

    pub fn duplicate(
        &mut self,
        node_ids: &[String],
        mark_ids: &[String],
    ) -> (Vec<String>, Vec<String>) {
        if self.readonly {
            return (Vec::new(), Vec::new());
        }
        let mut new_nodes = Vec::new();
        let mut new_marks = Vec::new();
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        let selected: BTreeSet<_> = node_ids.iter().cloned().collect();
        self.mutate(|graph| {
            for id in node_ids {
                let Some(node) = graph.node(id).cloned() else {
                    continue;
                };
                let new_id = graph.alloc_node_id();
                map.insert(id.clone(), new_id.clone());
                let mut copy = node;
                copy.id = new_id.clone();
                let point = snap_grid(Point::new(copy.position[0] + 32.0, copy.position[1] + 32.0));
                copy.position = [point.x, point.y];
                graph.nodes.push(copy);
                new_nodes.push(new_id);
            }
            let copied: Vec<Edge> = graph
                .edges
                .iter()
                .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
                .cloned()
                .collect();
            for mut edge in copied {
                let Some(from) = map.get(&edge.from) else {
                    continue;
                };
                let Some(to) = map.get(&edge.to) else {
                    continue;
                };
                edge.from = from.clone();
                edge.to = to.clone();
                edge.id = format!(
                    "{}:{}->{}:{}",
                    edge.from, edge.from_port, edge.to, edge.to_port
                );
                graph.edges.push(edge);
            }
            for id in mark_ids {
                let Some(mark) = graph.mark(id).cloned() else {
                    continue;
                };
                let new_id = graph.alloc_mark_id();
                let mut copy = mark;
                copy.id = new_id.clone();
                let point = snap_grid(Point::new(copy.position[0] + 32.0, copy.position[1] + 32.0));
                copy.position = [point.x, point.y];
                graph.marks.push(copy);
                new_marks.push(new_id);
            }
        });
        (new_nodes, new_marks)
    }

    pub fn connect(
        &mut self,
        from: &str,
        from_port: &str,
        to: &str,
        to_port: &str,
    ) -> Result<(), Reject> {
        if self.readonly {
            return Err(Reject::Type);
        }
        try_connect(&self.graph, from, from_port, to, to_port)?;
        let edge = Edge::between(from, from_port, to, to_port);
        self.mutate(|graph| graph.edges.push(edge));
        Ok(())
    }

    /// Remove a link as part of the open gesture. Cancel leaves it removed.
    /// Undo is what puts it back.
    pub fn detach(&mut self, edge_id: &str) -> Option<Edge> {
        if self.readonly {
            return None;
        }
        let edge = self
            .graph
            .edges
            .iter()
            .find(|edge| edge.id == edge_id)?
            .clone();
        let open = self.history.open.is_some();
        let remove = |graph: &mut Graph| {
            graph.edges.retain(|edge| edge.id != edge_id);
        };
        if open {
            remove(&mut self.graph);
        } else {
            self.mutate(remove);
        }
        Some(edge)
    }

    pub fn set_param(&mut self, node_id: &str, name: &str, value: Value) -> bool {
        if self.readonly || self.graph.node(node_id).is_none() {
            return false;
        }
        self.mutate(|graph| {
            if let Some(node) = graph.node_mut(node_id) {
                node.params.insert(name.to_string(), value);
            }
        });
        if name == "code" {
            self.sync_python_ports(node_id);
        }
        true
    }

    pub fn set_label(&mut self, node_id: &str, label: String) -> bool {
        if self.readonly {
            return false;
        }
        self.mutate(|graph| {
            if let Some(node) = graph.node_mut(node_id) {
                node.label = label;
            }
        });
        true
    }

    pub fn set_wired(&mut self, node_id: &str, param: &str, wired: bool) -> bool {
        if self.readonly {
            return false;
        }
        self.mutate(|graph| {
            let Some(node) = graph.node_mut(node_id) else {
                return;
            };
            if wired {
                node.wired.insert(param.to_string());
            } else {
                node.wired.remove(param);
                let port = param.to_string();
                let id = node.id.clone();
                graph
                    .edges
                    .retain(|edge| !(edge.to == id && edge.to_port == port));
            }
        });
        true
    }

    pub fn add_input(&mut self, node_id: &str) -> Option<String> {
        if self.readonly {
            return None;
        }
        let def = self.graph.node(node_id)?.def()?;
        if !def.can_add_input {
            return None;
        }
        let name = format!("in{}", self.graph.node(node_id)?.input_ports().len() + 1);
        let record = PortRecord {
            name: name.clone(),
            accepts: vec![TypeName::Dataframe, TypeName::Series],
            multi: false,
        };
        self.mutate(|graph| {
            if let Some(node) = graph.node_mut(node_id) {
                node.added_inputs.push(record);
            }
        });
        Some(name)
    }

    /// Replace a code block's inputs. Wires to ports that no longer exist are removed.
    pub fn set_python_inputs(&mut self, node_id: &str, inputs: Vec<PortRecord>) {
        let Some(node) = self.graph.node(node_id) else {
            return;
        };
        if node.feature != "python" {
            return;
        }
        let output = node
            .output
            .as_ref()
            .and_then(|port| port.accepts.first().copied())
            .unwrap_or(TypeName::Dataframe);
        self.set_python_ports(node_id, inputs, output);
    }

    pub fn set_python_ports(&mut self, node_id: &str, inputs: Vec<PortRecord>, output: TypeName) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            if let Some(node) = graph.node_mut(node_id) {
                if node.feature != "python" {
                    return;
                }
                let names: BTreeSet<_> = inputs.iter().map(|port| port.name.clone()).collect();
                node.inputs = Some(inputs);
                node.output = Some(PortRecord {
                    name: "out".to_string(),
                    accepts: vec![output],
                    multi: false,
                });
                let id = node.id.clone();
                graph
                    .edges
                    .retain(|edge| !(edge.to == id && !names.contains(&edge.to_port)));
            }
        });
    }

    fn sync_python_ports(&mut self, node_id: &str) {
        let Some(node) = self.graph.node(node_id) else {
            return;
        };
        if node.feature != "python" {
            return;
        }
        let names = crate::model::python_arg_names(&node.param_str("code"));
        if names.is_empty() {
            return;
        }
        let existing = node.input_ports();
        let mut inputs = Vec::new();
        for name in &names {
            if let Some(found) = existing.iter().find(|port| &port.name == name) {
                inputs.push(found.clone());
            } else {
                inputs.push(PortRecord {
                    name: name.clone(),
                    accepts: vec![TypeName::Dataframe],
                    multi: false,
                });
            }
        }
        let output = node
            .output
            .as_ref()
            .and_then(|port| port.accepts.first().copied())
            .unwrap_or(TypeName::Dataframe);
        self.set_python_ports(node_id, inputs, output);
    }

    pub fn set_port_type(&mut self, node_id: &str, port: &str, ty: TypeName) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            let Some(node) = graph.node_mut(node_id) else {
                return;
            };
            if let Some(found) = node
                .inputs
                .as_mut()
                .and_then(|inputs| inputs.iter_mut().find(|item| item.name == port))
            {
                found.accepts = vec![ty];
            }
        });
    }

    pub fn set_output_type(&mut self, node_id: &str, ty: TypeName) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            let Some(node) = graph.node_mut(node_id) else {
                return;
            };
            node.output = Some(PortRecord {
                name: "out".to_string(),
                accepts: vec![ty],
                multi: false,
            });
        });
    }

    pub fn add_mark(
        &mut self,
        kind: MarkKind,
        position: Point,
        size: Option<[f32; 2]>,
    ) -> Option<String> {
        if self.readonly {
            return None;
        }
        let id = self.graph.alloc_mark_id();
        let position = snap_grid(position);
        self.mutate(|graph| {
            graph.marks.push(Mark {
                id: id.clone(),
                kind,
                text: String::new(),
                position: [position.x, position.y],
                size,
                swatch: "neutral".to_string(),
                focus: None,
            });
        });
        Some(id)
    }

    pub fn set_mark_text(&mut self, id: &str, text: String) {
        self.mutate(|graph| {
            if let Some(mark) = graph.marks.iter_mut().find(|mark| mark.id == id) {
                mark.text = text;
            }
        });
    }

    pub fn set_mark_swatch(&mut self, id: &str, swatch: String) {
        self.mutate(|graph| {
            if let Some(mark) = graph.marks.iter_mut().find(|mark| mark.id == id) {
                mark.swatch = swatch;
            }
        });
    }

    pub fn set_mark_focus(&mut self, id: &str, focus: Option<String>) {
        self.mutate(|graph| {
            if let Some(mark) = graph.marks.iter_mut().find(|mark| mark.id == id) {
                mark.focus = focus;
            }
        });
    }

    pub fn paste_copies(
        &mut self,
        nodes: Vec<Node>,
        edges: Vec<Edge>,
        marks: Vec<Mark>,
    ) -> (Vec<String>, Vec<String>) {
        if self.readonly {
            return (Vec::new(), Vec::new());
        }
        let mut new_nodes = Vec::new();
        let mut new_marks = Vec::new();
        let mut map = BTreeMap::new();
        self.mutate(|graph| {
            for node in nodes {
                let new_id = graph.alloc_node_id();
                map.insert(node.id.clone(), new_id.clone());
                let mut copy = node;
                copy.id = new_id.clone();
                let point = snap_grid(Point::new(copy.position[0] + 32.0, copy.position[1] + 32.0));
                copy.position = [point.x, point.y];
                graph.nodes.push(copy);
                new_nodes.push(new_id);
            }
            for mut edge in edges {
                let Some(from) = map.get(&edge.from) else {
                    continue;
                };
                let Some(to) = map.get(&edge.to) else {
                    continue;
                };
                edge.from = from.clone();
                edge.to = to.clone();
                edge.id = format!(
                    "{}:{}->{}:{}",
                    edge.from, edge.from_port, edge.to, edge.to_port
                );
                graph.edges.push(edge);
            }
            for mark in marks {
                let new_id = graph.alloc_mark_id();
                let mut copy = mark;
                copy.id = new_id.clone();
                let point = snap_grid(Point::new(copy.position[0] + 32.0, copy.position[1] + 32.0));
                copy.position = [point.x, point.y];
                graph.marks.push(copy);
                new_marks.push(new_id);
            }
        });
        (new_nodes, new_marks)
    }

    pub fn nudge(&mut self, node_ids: &[String], mark_ids: &[String], dx: f32, dy: f32) {
        if self.readonly || (dx == 0.0 && dy == 0.0) {
            return;
        }
        self.mutate(|graph| {
            for node in &mut graph.nodes {
                if node_ids.iter().any(|id| id == &node.id) {
                    node.position[0] += dx;
                    node.position[1] += dy;
                }
            }
            for mark in &mut graph.marks {
                if mark_ids.iter().any(|id| id == &mark.id) {
                    mark.position[0] += dx;
                    mark.position[1] += dy;
                }
            }
        });
    }

    pub fn reorder(&mut self, node_ids: &[String], mark_ids: &[String], front: bool) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            let node_set: BTreeSet<_> = node_ids.iter().cloned().collect();
            let mark_set: BTreeSet<_> = mark_ids.iter().cloned().collect();
            let (mut keep_nodes, mut move_nodes): (Vec<_>, Vec<_>) = graph
                .nodes
                .drain(..)
                .partition(|node| !node_set.contains(&node.id));
            if front {
                keep_nodes.append(&mut move_nodes);
                graph.nodes = keep_nodes;
            } else {
                move_nodes.append(&mut keep_nodes);
                graph.nodes = move_nodes;
            }
            let (mut keep_marks, mut move_marks): (Vec<_>, Vec<_>) = graph
                .marks
                .drain(..)
                .partition(|mark| !mark_set.contains(&mark.id));
            if front {
                keep_marks.append(&mut move_marks);
                graph.marks = keep_marks;
            } else {
                move_marks.append(&mut keep_marks);
                graph.marks = move_marks;
            }
        });
    }

    pub fn raise_mark(&mut self, id: &str) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            if let Some(index) = graph.marks.iter().position(|mark| mark.id == id) {
                let mark = graph.marks.remove(index);
                graph.marks.push(mark);
            }
        });
    }

    pub fn resize_mark(&mut self, id: &str, size: [f32; 2]) {
        let open = self.history.open.is_some();
        let apply = |graph: &mut Graph| {
            if let Some(mark) = graph.marks.iter_mut().find(|mark| mark.id == id) {
                mark.size = Some(size);
            }
        };
        if open {
            apply(&mut self.graph);
        } else {
            self.mutate(apply);
        }
    }

    pub fn set_presentation(&mut self, node_id: &str, presentation: crate::model::Presentation) {
        self.mutate(|graph| {
            if let Some(node) = graph.node_mut(node_id) {
                node.presentation = Some(presentation);
            }
        });
    }

    pub fn set_mapping_pair(&mut self, node_id: &str, index: usize, from: String, to: String) {
        self.mutate(|graph| {
            let Some(node) = graph.node_mut(node_id) else {
                return;
            };
            let entry = node
                .params
                .entry(crate::model::HEADER_PAIRS.to_string())
                .or_insert_with(|| json!([]));
            if !entry.is_array() {
                *entry = json!([]);
            }
            let array = entry.as_array_mut().expect("array");
            while array.len() <= index {
                array.push(json!({"from": "", "to": ""}));
            }
            array[index] = json!({"from": from, "to": to});
        });
    }

    /// Drop a feature onto an edge. The edge is replaced only when the new
    /// feature's input accepts the upstream type and its output feeds the old target.
    pub fn insert_on_edge(
        &mut self,
        edge_id: &str,
        feature_id: &str,
        position: Point,
    ) -> Result<String, Reject> {
        if self.readonly {
            return Err(Reject::Type);
        }
        let edge = self
            .graph
            .edges
            .iter()
            .find(|edge| edge.id == edge_id)
            .cloned()
            .ok_or(Reject::Type)?;
        let id = self.graph.alloc_node_id();
        let position = snap_grid(position);
        let mut trial = self.graph.clone();
        trial.edges.retain(|item| item.id != edge.id);
        trial
            .nodes
            .push(Node::new(id.clone(), feature_id, position));
        let new_node = trial.node(&id).ok_or(Reject::Type)?.clone();
        let input = new_node
            .input_ports()
            .into_iter()
            .next()
            .ok_or(Reject::Type)?;
        let output = new_node.output_port().ok_or(Reject::Type)?;
        try_connect(&trial, &edge.from, &edge.from_port, &id, &input.name)?;
        trial
            .edges
            .push(Edge::between(&edge.from, &edge.from_port, &id, &input.name));
        try_connect(&trial, &id, &output.name, &edge.to, &edge.to_port)?;
        let downstream = Edge::between(&id, &output.name, &edge.to, &edge.to_port);
        let upstream = Edge::between(&edge.from, &edge.from_port, &id, &input.name);
        self.mutate(|graph| {
            graph
                .nodes
                .push(Node::new(id.clone(), feature_id, position));
            graph.edges.retain(|item| item.id != edge.id);
            graph.edges.push(upstream);
            graph.edges.push(downstream);
        });
        Ok(id)
    }

    pub fn features_accepting(&self, ty: TypeName) -> Vec<&'static FeatureDef> {
        feature::CATALOG
            .iter()
            .filter(|def| {
                def.inputs.iter().any(|port| port.accepts.contains(&ty))
                    || def
                        .params
                        .iter()
                        .any(|param| param.wireable && param.wire_types.contains(&ty))
                    || (def.inputs_are_user
                        && (ty == TypeName::Dataframe || ty == TypeName::Scalar))
            })
            .collect()
    }
}

fn can_write(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match path.metadata() {
            Ok(meta) => meta.permissions().mode() & 0o222 != 0,
            Err(_) => true,
        }
    }
    #[cfg(not(unix))]
    {
        path.metadata()
            .map(|meta| !meta.permissions().readonly())
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::Reject;

    fn doc_with_chain() -> (Document, String, String) {
        let mut doc = Document::new();
        let a = doc.add_node("read_csv", Point::new(10.0, 10.0)).unwrap();
        let b = doc.add_node("head", Point::new(300.0, 10.0)).unwrap();
        doc.connect(&a, "out", &b, "in").unwrap();
        (doc, a, b)
    }

    #[test]
    fn undo_removes_a_wire_and_redo_puts_it_back() {
        let (mut doc, _, _) = doc_with_chain();
        assert_eq!(doc.graph().edges.len(), 1);
        assert!(doc.undo());
        assert!(doc.graph().edges.is_empty());
        assert!(doc.redo());
        assert_eq!(doc.graph().edges.len(), 1);
    }

    #[test]
    fn a_cancelled_detach_stays_removed_until_undo() {
        let (mut doc, _, _) = doc_with_chain();
        let edge_id = doc.graph().edges[0].id.clone();
        doc.begin_gesture();
        doc.detach(&edge_id);
        doc.commit_gesture();
        assert!(doc.graph().edges.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.graph().edges[0].id, edge_id);
    }

    #[test]
    fn cancelling_a_drag_that_never_detached_writes_nothing() {
        let (mut doc, _, _) = doc_with_chain();
        let before = doc.can_undo();
        doc.begin_gesture();
        doc.cancel_gesture();
        assert_eq!(doc.can_undo(), before);
        assert_eq!(doc.graph().edges.len(), 1);
    }

    #[test]
    fn delete_then_undo_does_not_leave_a_dangling_edge() {
        let (mut doc, a, b) = doc_with_chain();
        doc.delete(std::slice::from_ref(&b), &[]);
        assert!(doc.graph().edges.is_empty());
        assert!(doc.graph().node(&a).is_some());
        assert!(doc.undo());
        assert!(doc.graph().node(&b).is_some());
        assert_eq!(doc.graph().edges.len(), 1);
        assert!(doc.graph().node(&doc.graph().edges[0].to).is_some());
    }

    #[test]
    fn duplicate_copies_internal_wires_only() {
        let (mut doc, a, b) = doc_with_chain();
        let c = doc.add_node("query", Point::new(500.0, 10.0)).unwrap();
        doc.connect(&b, "out", &c, "in").unwrap();
        let (copies, _) = doc.duplicate(&[a.clone(), b.clone()], &[]);
        assert_eq!(copies.len(), 2);
        let copy_a = &copies[0];
        let copy_b = &copies[1];
        let internal = doc
            .graph()
            .edges
            .iter()
            .any(|edge| &edge.from == copy_a && &edge.to == copy_b);
        assert!(internal);
        let dangling = doc.graph().edges.iter().any(|edge| {
            doc.graph().node(&edge.from).is_none() || doc.graph().node(&edge.to).is_none()
        });
        assert!(!dangling);
        let leaked = doc.graph().edges.iter().any(|edge| {
            (&edge.from == copy_b && edge.to == c)
                || (&edge.to == copy_a && edge.from != *copy_b && edge.from != a)
        });
        assert!(!leaked);
    }

    #[test]
    fn a_card_drag_is_one_undo_record() {
        let mut doc = Document::new();
        let id = doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        doc.begin_gesture();
        doc.place(
            &BTreeMap::from([(id.clone(), Point::new(3.0, 3.0))]),
            &BTreeMap::new(),
            false,
        );
        doc.place(
            &BTreeMap::from([(id.clone(), Point::new(40.0, 8.0))]),
            &BTreeMap::new(),
            true,
        );
        doc.commit_gesture();
        assert_eq!(doc.graph().node(&id).unwrap().position, [48.0, 16.0]);
        assert!(doc.undo());
        assert_eq!(doc.graph().node(&id).unwrap().position, [0.0, 0.0]);
        assert!(doc.graph().node(&id).is_some());
    }

    #[test]
    fn insert_on_a_wire_requires_both_types() {
        let (mut doc, a, b) = doc_with_chain();
        let edge = doc.graph().edges[0].id.clone();
        let id = doc
            .insert_on_edge(&edge, "query", Point::new(160.0, 16.0))
            .unwrap();
        assert_eq!(doc.graph().edges.len(), 2);
        assert!(doc.graph().edges.iter().any(|edge| edge.to == id));
        assert!(
            doc.graph()
                .edges
                .iter()
                .any(|edge| edge.from == id && edge.to == b)
        );
        assert!(doc.undo());
        assert_eq!(doc.graph().edges.len(), 1);
        assert_eq!(doc.graph().edges[0].from, a);

        let mut doc = Document::new();
        let source = doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        let target = doc
            .add_node("change_headers", Point::new(400.0, 0.0))
            .unwrap();
        doc.connect(&source, "out", &target, "in").unwrap();
        let edge = doc.graph().edges[0].id.clone();
        let err = doc
            .insert_on_edge(&edge, "column", Point::new(160.0, 16.0))
            .unwrap_err();
        assert_eq!(err, Reject::Type);
        assert_eq!(doc.graph().nodes.len(), 2);
    }

    #[test]
    fn param_edits_share_one_transaction() {
        let mut doc = Document::new();
        let id = doc.add_node("head", Point::new(0.0, 0.0)).unwrap();
        doc.begin_gesture();
        doc.set_param(&id, "n", json!(10));
        doc.set_param(&id, "n", json!(3));
        doc.commit_gesture();
        assert_eq!(doc.graph().node(&id).unwrap().param_i64("n"), Some(3));
        assert!(doc.undo());
        assert_eq!(doc.graph().node(&id).unwrap().param_i64("n"), Some(5));
    }

    #[test]
    fn nudge_moves_only_the_selection() {
        let (mut doc, a, b) = doc_with_chain();
        let before = doc.graph().node(&b).unwrap().position;
        doc.nudge(&[a], &[], 16.0, 0.0);
        assert_eq!(doc.graph().node(&b).unwrap().position, before);
        assert!(doc.undo());
    }

    #[test]
    fn python_inputs_replace_ports_and_drop_wires_that_no_longer_fit() {
        let mut doc = Document::new();
        let source = doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        let code = doc.add_node("python", Point::new(240.0, 0.0)).unwrap();
        doc.set_python_inputs(
            &code,
            vec![PortRecord {
                name: "frame".into(),
                accepts: vec![TypeName::Dataframe],
                multi: false,
            }],
        );
        doc.connect(&source, "out", &code, "frame").unwrap();
        doc.set_python_inputs(
            &code,
            vec![PortRecord {
                name: "rows".into(),
                accepts: vec![TypeName::Dataframe],
                multi: false,
            }],
        );
        assert!(doc.graph().edges.is_empty());
        assert_eq!(
            doc.graph().node(&code).unwrap().input_ports()[0].name,
            "rows"
        );
    }

    #[test]
    fn a_typed_run_signature_keeps_the_argument_name() {
        let names =
            crate::model::python_arg_names("def run(amount: object, region):\n    return amount\n");
        assert_eq!(names, vec!["amount".to_string(), "region".to_string()]);
    }
}
