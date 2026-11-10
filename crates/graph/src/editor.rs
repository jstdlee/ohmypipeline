use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::connect::{Reject, try_connect};
use crate::feature::{self, FeatureDef};
use crate::file::{self, FileError};
use crate::geom::{Point, snap_grid};
use crate::model::{
    Edge, Graph, Mark, MarkKind, Node, PortRecord, Variable, bounds_of, frame_members,
};
use crate::types::TypeName;

#[derive(Clone, Debug)]
struct Snapshot {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    marks: Vec<Mark>,
    frames: Vec<crate::model::Frame>,
    variables: Vec<crate::model::Variable>,
}

impl Snapshot {
    fn capture(graph: &Graph) -> Self {
        Self {
            nodes: graph.nodes.clone(),
            edges: graph.edges.clone(),
            marks: graph.marks.clone(),
            frames: graph.frames.clone(),
            variables: graph.variables.clone(),
        }
    }

    fn same(&self, graph: &Graph) -> bool {
        self.nodes == graph.nodes
            && self.edges == graph.edges
            && self.marks == graph.marks
            && self.frames == graph.frames
            && self.variables == graph.variables
    }

    fn restore(&self, graph: &mut Graph) {
        graph.nodes = self.nodes.clone();
        graph.edges = self.edges.clone();
        graph.marks = self.marks.clone();
        graph.frames = self.frames.clone();
        graph.variables = self.variables.clone();
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

    /// Place nodes at absolute positions as one undo step. Used by
    /// auto-layout, which computes every new position up front.
    pub fn place_many(&mut self, positions: &BTreeMap<String, Point>) -> bool {
        if self.readonly || positions.is_empty() {
            return false;
        }
        self.mutate(|graph| {
            for node in &mut graph.nodes {
                if let Some(point) = positions.get(&node.id) {
                    node.position = [point.x, point.y];
                }
            }
        });
        true
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

    /// Stores a fit without an undo step. A preview fit must not become an edit the user undoes away by accident, and it is not part of the run fingerprint.
    pub fn set_fitted(&mut self, node_id: &str, fitted: Option<serde_json::Value>) {
        if let Some(node) = self.graph.node_mut(node_id) {
            node.fitted = fitted;
            self.dirty = true;
        }
    }

    pub fn set_notes(&mut self, node_id: &str, notes: String) -> bool {
        if self.readonly {
            return false;
        }
        self.mutate(|graph| {
            if let Some(node) = graph.node_mut(node_id) {
                node.notes = if notes.is_empty() { None } else { Some(notes) };
            }
        });
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

    pub fn add_frame(&mut self, title: String, position: Point, size: [f32; 2]) -> Option<String> {
        if self.readonly {
            return None;
        }
        let id = self.graph.alloc_frame_id();
        let position = snap_grid(position);
        self.mutate(|graph| {
            graph.frames.push(crate::model::Frame {
                id: id.clone(),
                title,
                swatch: "neutral".to_string(),
                position: [position.x, position.y],
                size,
                collapsed: false,
            });
        });
        Some(id)
    }

    /// Size a frame around the selection. The top grows by `HEADER_ALLOWANCE`
    /// so a frame placed over a group shows a header above the cards.
    pub fn frame_around(
        &mut self,
        node_ids: &[String],
        mark_ids: &[String],
        padding: f32,
    ) -> Option<String> {
        if self.readonly {
            return None;
        }
        let ids: Vec<String> = node_ids.iter().chain(mark_ids.iter()).cloned().collect();
        if ids.is_empty() {
            return None;
        }
        let bounds = bounds_of(&self.graph, &ids, estimated_card_height)?;
        let position = snap_grid(Point::new(
            bounds.x - padding,
            bounds.y - HEADER_ALLOWANCE - padding,
        ));
        let size = [
            (bounds.w + 2.0 * padding).max(MIN_FRAME_SIZE[0]),
            (bounds.h + HEADER_ALLOWANCE + 2.0 * padding).max(MIN_FRAME_SIZE[1]),
        ];
        let id = self.graph.alloc_frame_id();
        self.mutate(|graph| {
            graph.frames.push(crate::model::Frame {
                id: id.clone(),
                title: String::new(),
                swatch: "neutral".to_string(),
                position: [position.x, position.y],
                size,
                collapsed: false,
            });
        });
        Some(id)
    }

    /// Move a frame and its members in one gesture so the group stays whole.
    pub fn move_frame(&mut self, id: &str, delta: Point) {
        if self.readonly {
            return;
        }
        let members = frame_members(&self.graph, id);
        self.mutate(|graph| {
            if let Some(frame) = graph.frames.iter_mut().find(|frame| frame.id == id) {
                frame.position[0] += delta.x;
                frame.position[1] += delta.y;
            }
            for node in &mut graph.nodes {
                if members.iter().any(|member| member == &node.id) {
                    node.position[0] += delta.x;
                    node.position[1] += delta.y;
                }
            }
        });
    }

    pub fn resize_frame(&mut self, id: &str, size: [f32; 2]) {
        let open = self.history.open.is_some();
        let apply = |graph: &mut Graph| {
            if let Some(frame) = graph.frames.iter_mut().find(|frame| frame.id == id) {
                frame.size = [
                    size[0].max(MIN_FRAME_SIZE[0]),
                    size[1].max(MIN_FRAME_SIZE[1]),
                ];
            }
        };
        if open {
            apply(&mut self.graph);
        } else {
            self.mutate(apply);
        }
    }

    pub fn set_frame_title(&mut self, id: &str, title: String) {
        self.mutate(|graph| {
            if let Some(frame) = graph.frames.iter_mut().find(|frame| frame.id == id) {
                frame.title = title;
            }
        });
    }

    pub fn set_frame_swatch(&mut self, id: &str, swatch: String) {
        self.mutate(|graph| {
            if let Some(frame) = graph.frames.iter_mut().find(|frame| frame.id == id) {
                frame.swatch = swatch;
            }
        });
    }

    pub fn set_frame_collapsed(&mut self, id: &str, collapsed: bool) {
        self.mutate(|graph| {
            if let Some(frame) = graph.frames.iter_mut().find(|frame| frame.id == id) {
                frame.collapsed = collapsed;
            }
        });
    }

    /// Remove frames only. Frame members stay in the document.
    pub fn delete_frames(&mut self, ids: &[String]) {
        if self.readonly || ids.is_empty() {
            return;
        }
        self.mutate(|graph| {
            let set: BTreeSet<_> = ids.iter().cloned().collect();
            graph.frames.retain(|frame| !set.contains(&frame.id));
        });
    }

    /// Insert a bend point into a wire at `index`. Past the end appends.
    pub fn add_reroute(&mut self, edge_id: &str, index: usize, point: Point) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            if let Some(edge) = graph.edges.iter_mut().find(|edge| edge.id == edge_id) {
                let index = index.min(edge.reroutes.len());
                edge.reroutes.insert(index, [point.x, point.y]);
            }
        });
    }

    pub fn move_reroute(&mut self, edge_id: &str, index: usize, point: Point) {
        let open = self.history.open.is_some();
        let apply = |graph: &mut Graph| {
            if let Some(reroute) = graph
                .edges
                .iter_mut()
                .find(|edge| edge.id == edge_id)
                .and_then(|edge| edge.reroutes.get_mut(index))
            {
                *reroute = [point.x, point.y];
            }
        };
        if open {
            apply(&mut self.graph);
        } else {
            self.mutate(apply);
        }
    }

    pub fn remove_reroute(&mut self, edge_id: &str, index: usize) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            for edge in &mut graph.edges {
                if edge.id != edge_id {
                    continue;
                }
                if index < edge.reroutes.len() {
                    edge.reroutes.remove(index);
                }
            }
        });
    }

    /// Drop every bend point of a wire. The wire stays.
    pub fn clear_reroutes(&mut self, edge_id: &str) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            if let Some(edge) = graph.edges.iter_mut().find(|edge| edge.id == edge_id) {
                edge.reroutes.clear();
            }
        });
    }

    /// Insert by name or replace the existing entry with the same name.
    pub fn set_variable(&mut self, variable: Variable) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            if let Some(found) = graph
                .variables
                .iter_mut()
                .find(|found| found.name == variable.name)
            {
                *found = variable;
            } else {
                graph.variables.push(variable);
            }
        });
    }

    pub fn rename_variable(&mut self, old: &str, new: &str) -> bool {
        if self.readonly || old == new || new.is_empty() || !crate::file::is_variable_name(new) {
            return false;
        }
        let taken = self
            .graph
            .variables
            .iter()
            .any(|variable| variable.name == new);
        if taken {
            return false;
        }
        self.mutate(|graph| {
            if let Some(variable) = graph
                .variables
                .iter_mut()
                .find(|variable| variable.name == old)
            {
                variable.name = new.to_string();
            }
        });
        true
    }

    pub fn remove_variable(&mut self, name: &str) {
        if self.readonly {
            return;
        }
        self.mutate(|graph| {
            graph.variables.retain(|variable| variable.name != name);
        });
    }

    /// Window layout only. It marks the document dirty but writes no undo step.
    pub fn set_view(&mut self, view: crate::model::View) {
        self.graph.view = view;
        self.dirty = true;
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

    /// Thread an existing, unconnected node into a wire. The node keeps its id
    /// and parameters; the wire is replaced by the two edges through it.
    pub fn insert_node_on_edge(&mut self, edge_id: &str, node_id: &str) -> Result<(), Reject> {
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
        let node = self.graph.node(node_id).ok_or(Reject::Type)?.clone();
        if self
            .graph
            .edges
            .iter()
            .any(|edge| edge.from == node.id || edge.to == node.id)
        {
            return Err(Reject::Type);
        }
        let input = node.input_ports().into_iter().next().ok_or(Reject::Type)?;
        let output = node.output_port().ok_or(Reject::Type)?;
        let mut trial = self.graph.clone();
        trial.edges.retain(|item| item.id != edge.id);
        try_connect(&trial, &edge.from, &edge.from_port, node_id, &input.name)?;
        try_connect(&trial, node_id, &output.name, &edge.to, &edge.to_port)?;
        let upstream = Edge::between(&edge.from, &edge.from_port, node_id, &input.name);
        let downstream = Edge::between(node_id, &output.name, &edge.to, &edge.to_port);
        self.mutate(|graph| {
            graph.edges.retain(|item| item.id != edge.id);
            graph.edges.push(upstream);
            graph.edges.push(downstream);
        });
        Ok(())
    }

    /// Point every picked wire at a new output port. Each wire moves only when
    /// the normal connect rules accept it; the rest stay where they were.
    /// Returns how many moved and how many were rejected.
    pub fn rewire_sources(
        &mut self,
        edge_ids: &[String],
        new_from: &str,
        new_port: &str,
    ) -> (usize, usize) {
        if self.readonly || edge_ids.is_empty() {
            return (0, edge_ids.len());
        }
        let picked: BTreeSet<String> = edge_ids.iter().cloned().collect();
        // Work on a copy with the picked wires removed, so a batch of moves
        // never collides with itself (a target input is free until a wire
        // actually moves onto it). Candidates are validated against the copy
        // plus the wires already accepted, never against themselves.
        let mut trial = self.graph.clone();
        trial.edges.retain(|edge| !picked.contains(&edge.id));
        let mut accepted: Vec<Edge> = Vec::new();
        let mut moved = 0usize;
        let mut rejected = 0usize;
        for edge in self
            .graph
            .edges
            .iter()
            .filter(|edge| picked.contains(&edge.id))
        {
            // A wire onto its own source would be a self loop, and checking
            // one recurses forever in `output_types`.
            if edge.to == new_from {
                rejected += 1;
                continue;
            }
            let mut candidate = Edge::between(new_from, new_port, &edge.to, &edge.to_port);
            candidate.reroutes = edge.reroutes.clone();
            if try_connect(&trial, new_from, new_port, &edge.to, &edge.to_port).is_ok() {
                trial.edges.push(candidate.clone());
                accepted.push(candidate);
                moved += 1;
            } else {
                rejected += 1;
            }
        }
        if moved > 0 {
            let apply = |graph: &mut Graph| {
                graph.edges.retain(|edge| !picked.contains(&edge.id));
                graph.edges.extend(accepted.clone());
            };
            if self.history.open.is_some() {
                apply(&mut self.graph);
            } else {
                self.mutate(apply);
            }
        }
        (moved, rejected)
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

    /// Features a fresh node of which could produce `ty`. The mirror of
    /// [`Document::features_accepting`] for a drag that ends on an input.
    pub fn features_producing(&self, ty: TypeName) -> Vec<&'static FeatureDef> {
        feature::CATALOG
            .iter()
            .filter(|def| def.output == Some(ty) || def.possible_outputs.contains(&ty))
            .collect()
    }
}

/// Header strip a frame draws on top of its members.
const HEADER_ALLOWANCE: f32 = 36.0;
/// Smallest frame the inspector keeps on screen.
const MIN_FRAME_SIZE: [f32; 2] = [120.0, 60.0];

/// The file sizes cards exactly, but the card layout lives in `canvas`.
/// Frame sizing only needs a generous estimate: port rows plus a body
/// allowance, always a little taller than the canvas would draw.
fn estimated_card_height(node: &Node) -> f32 {
    let rows = node
        .input_ports()
        .len()
        .max(usize::from(node.output_port().is_some()))
        .max(1) as f32;
    28.0 + 16.0 + 8.0 + 22.0 * rows + 8.0 + 96.0
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
    use crate::model::VarKind;

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

    fn a_document_with_a_frame_and_members() -> (Document, String, String, String) {
        let mut doc = Document::new();
        let a = doc.add_node("read_csv", Point::new(64.0, 64.0)).unwrap();
        let b = doc.add_node("head", Point::new(320.0, 64.0)).unwrap();
        let c = doc.add_node("query", Point::new(640.0, 400.0)).unwrap();
        let id = doc
            .frame_around(&[a.clone(), b.clone()], &[], 16.0)
            .unwrap();
        (doc, id, b, c)
    }

    #[test]
    fn frame_around_sizes_a_rectangle_over_the_selection() {
        let (doc, frame_id, _, _) = a_document_with_a_frame_and_members();
        let frame = &doc.graph().frames[0];
        // The two nodes covered by the selection are members, the third is not.
        let expected: Vec<String> = doc
            .graph()
            .nodes
            .iter()
            .take(2)
            .map(|node| node.id.clone())
            .collect();
        assert_eq!(frame_members(doc.graph(), &frame_id), expected);
        // The header allowance lifts the rectangle above the cards.
        let card = &doc.graph().nodes[0];
        assert!(frame.position[0] <= card.position[0]);
        assert!(frame.position[1] < card.position[1]);
        assert!(!frame.collapsed);
        assert_eq!(frame.swatch, "neutral");
    }

    #[test]
    fn set_view_marks_dirty_but_writes_no_undo_step() {
        let mut doc = Document::from_graph(Graph::default());
        assert!(!doc.is_dirty());
        assert!(!doc.can_undo());
        doc.set_view(crate::model::View {
            minimap: true,
            ..crate::model::View::default()
        });
        assert!(doc.is_dirty());
        assert!(!doc.can_undo());
        doc.undo();
        assert!(doc.graph().view.minimap, "undo must not revert the view");
    }

    #[test]
    fn adding_a_frame_is_one_undo_step() {
        let (mut doc, _, _, _) = a_document_with_a_frame_and_members();
        assert_eq!(doc.graph().frames.len(), 1);
        assert!(doc.undo());
        assert!(doc.graph().frames.is_empty());
        assert!(doc.redo());
        assert_eq!(doc.graph().frames.len(), 1);
    }

    #[test]
    fn moving_a_frame_moves_its_members_and_undo_restores_both() {
        let (mut doc, frame_id, b, c) = a_document_with_a_frame_and_members();
        let frame_before = doc.graph().frames[0].position;
        let inside_before = doc.graph().node(&b).unwrap().position;
        let outside_before = doc.graph().node(&c).unwrap().position;
        doc.move_frame(&frame_id, Point::new(-16.0, 32.0));
        let frame_after = doc.graph().frames[0].position;
        assert_eq!(
            frame_after,
            [frame_before[0] - 16.0, frame_before[1] + 32.0]
        );
        assert_eq!(
            doc.graph().node(&b).unwrap().position,
            [inside_before[0] - 16.0, inside_before[1] + 32.0]
        );
        assert_eq!(doc.graph().node(&c).unwrap().position, outside_before);
        assert!(doc.undo());
        assert_eq!(doc.graph().frames[0].position, frame_before);
        assert_eq!(doc.graph().node(&b).unwrap().position, inside_before);
        assert_eq!(doc.graph().node(&c).unwrap().position, outside_before);
    }

    #[test]
    fn resizing_a_frame_clamps_to_a_minimum_and_undo_restores_it() {
        let (mut doc, frame_id, _, _) = a_document_with_a_frame_and_members();
        let before = doc.graph().frames[0].size;
        doc.resize_frame(&frame_id, [10.0, 10.0]);
        assert_eq!(doc.graph().frames[0].size, [120.0, 60.0]);
        doc.resize_frame(&frame_id, [480.0, 320.0]);
        assert_eq!(doc.graph().frames[0].size, [480.0, 320.0]);
        assert!(doc.undo());
        assert_eq!(doc.graph().frames[0].size, [120.0, 60.0]);
        assert!(doc.undo());
        assert_eq!(doc.graph().frames[0].size, before);
    }

    #[test]
    fn collapsing_a_frame_is_one_undo_step() {
        let (mut doc, frame_id, _, _) = a_document_with_a_frame_and_members();
        doc.set_frame_collapsed(&frame_id, true);
        assert!(doc.graph().frames[0].collapsed);
        assert!(doc.undo());
        assert!(!doc.graph().frames[0].collapsed);
        doc.set_frame_title(&frame_id, "Triage".to_string());
        doc.set_frame_swatch(&frame_id, "c4".to_string());
        // Title and swatch are two separate edits; the first undo only
        // reverts the swatch.
        assert!(doc.undo());
        assert_eq!(doc.graph().frames[0].title, "Triage");
        assert_eq!(doc.graph().frames[0].swatch, "neutral");
        assert!(doc.undo());
        assert_eq!(doc.graph().frames[0].title, "");
        assert!(!doc.graph().frames[0].collapsed);
    }

    #[test]
    fn deleting_frames_leaves_the_members() {
        let (mut doc, frame_id, b, c) = a_document_with_a_frame_and_members();
        doc.delete_frames(&[frame_id.clone(), "f99".to_string()]);
        assert!(doc.graph().frames.is_empty());
        assert!(doc.graph().node(&b).is_some());
        assert!(doc.graph().node(&c).is_some());
        assert!(doc.undo());
        assert!(doc.graph().frames.iter().any(|f| f.id == frame_id));
    }

    #[test]
    fn deleting_a_node_removes_its_edges_but_keeps_frames() {
        let (mut doc, frame_id, b, _) = a_document_with_a_frame_and_members();
        let a = doc.graph().nodes[0].id.clone();
        doc.connect(&a, "out", &b, "in").unwrap();
        assert!(!doc.graph().edges.is_empty());
        doc.delete(std::slice::from_ref(&a), &[]);
        assert!(doc.graph().edges.is_empty());
        assert!(doc.graph().frames.iter().any(|frame| frame.id == frame_id));
    }

    #[test]
    fn add_move_and_remove_reroutes_are_each_one_undo_step() {
        let (mut doc, _, _) = doc_with_chain();
        let edge = doc.graph().edges[0].id.clone();
        doc.add_reroute(&edge, 0, Point::new(160.0, 100.0));
        assert_eq!(doc.graph().edges[0].reroutes, [[160.0, 100.0]]);
        doc.add_reroute(&edge, 99, Point::new(240.0, 100.0));
        assert_eq!(doc.graph().edges[0].reroutes.len(), 2);
        assert!(doc.undo());
        assert_eq!(doc.graph().edges[0].reroutes, [[160.0, 100.0]]);
        doc.move_reroute(&edge, 0, Point::new(208.0, 128.0));
        assert_eq!(doc.graph().edges[0].reroutes, [[208.0, 128.0]]);
        assert!(doc.undo());
        assert_eq!(doc.graph().edges[0].reroutes, [[160.0, 100.0]]);
        doc.remove_reroute(&edge, 0);
        assert!(doc.graph().edges[0].reroutes.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.graph().edges[0].reroutes, [[160.0, 100.0]]);
        doc.remove_reroute(&edge, 7);
        assert_eq!(doc.graph().edges[0].reroutes, [[160.0, 100.0]]);
    }

    #[test]
    fn variables_undo_one_step_at_a_time() {
        let mut doc = Document::new();
        doc.set_variable(crate::model::Variable {
            name: "city".into(),
            kind: crate::model::VarKind::String,
            value: serde_json::json!("Paris"),
        });
        doc.set_variable(crate::model::Variable {
            name: "top".into(),
            kind: crate::model::VarKind::Int,
            value: serde_json::json!(5),
        });
        assert_eq!(doc.graph().variables.len(), 2);
        doc.set_variable(crate::model::Variable {
            name: "city".into(),
            kind: crate::model::VarKind::Path,
            value: serde_json::json!("orders.csv"),
        });
        assert_eq!(
            (
                doc.graph().variables[0].kind,
                doc.graph().variables[0].value.clone()
            ),
            (crate::model::VarKind::Path, serde_json::json!("orders.csv"))
        );
        assert!(doc.undo());
        assert_eq!(doc.graph().variables[0].value, serde_json::json!("Paris"));
        assert!(
            !doc.rename_variable("top", "city"),
            "a rename must not collide"
        );
        assert!(doc.rename_variable("top", "row_cap"));
        assert!(doc.graph().variables.iter().any(|v| v.name == "row_cap"));
        assert!(doc.undo());
        assert!(doc.graph().variables.iter().any(|v| v.name == "top"));
        doc.remove_variable("top");
        assert_eq!(doc.graph().variables.len(), 1);
        assert!(doc.undo());
        assert_eq!(doc.graph().variables.len(), 2);
    }

    #[test]
    fn frame_ids_never_clash() {
        let mut doc = Document::new();
        let one = doc
            .add_frame("One".into(), Point::new(0.0, 0.0), [300.0, 200.0])
            .unwrap();
        let two = doc
            .add_frame("Two".into(), Point::new(0.0, 0.0), [300.0, 200.0])
            .unwrap();
        assert_ne!(one, two);
        assert_eq!(one, "f1");
        assert_eq!(two, "f2");
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

    fn doc_with_fan() -> (Document, String, String, String, Vec<String>) {
        let mut doc = Document::new();
        let a = doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        let b = doc.add_node("head", Point::new(300.0, 0.0)).unwrap();
        let c = doc.add_node("query", Point::new(300.0, 200.0)).unwrap();
        let d = doc.add_node("read_csv", Point::new(600.0, 0.0)).unwrap();
        let _ = doc.connect(&a, "out", &b, "in");
        let _ = doc.connect(&a, "out", &c, "in");
        let ids = doc.graph().edges.iter().map(|e| e.id.clone()).collect();
        (doc, a, b, d, ids)
    }

    #[test]
    fn rewire_sources_moves_every_wire_and_undo_restores_them() {
        let (mut doc, a, b, d, ids) = doc_with_fan();
        let (moved, rejected) = doc.rewire_sources(&ids, &d, "out");
        assert_eq!((moved, rejected), (2, 0));
        let edges = doc.graph().edges.clone();
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|edge| edge.from == d));
        assert!(doc.graph().edges.iter().any(|edge| edge.to == b));
        assert!(doc.undo());
        let restored = doc.graph().edges.clone();
        assert!(restored.iter().all(|edge| edge.from == a));
        assert_eq!(restored.len(), 2);
    }

    #[test]
    fn rewire_sources_keeps_wires_the_rules_reject() {
        let (mut doc, _, b, _, ids) = doc_with_fan();
        // Re-sourcing each wire onto its own target would cycle.
        let targets: Vec<String> = doc
            .graph()
            .edges
            .iter()
            .map(|edge| edge.to.clone())
            .collect();
        let mut total = (0usize, 0usize);
        for (edge, target) in ids.iter().zip(targets.iter()) {
            let (moved, rejected) = doc.rewire_sources(std::slice::from_ref(edge), target, "out");
            total.0 += moved;
            total.1 += rejected;
        }
        assert_eq!(total, (0, 2));
        assert_eq!(doc.graph().edges.len(), 2);
        assert!(doc.graph().edges.iter().any(|edge| edge.to == *b));
        // A read-only document rejects the whole batch too.
        let mut frozen = Document::from_graph(doc.graph().clone());
        frozen.readonly = true;
        assert_eq!(frozen.rewire_sources(&ids, &b, "out"), (0, ids.len()));
    }

    #[test]
    fn rewire_sources_moves_only_the_picked_wires() {
        let (mut doc, a, _, d, ids) = doc_with_fan();
        let first = vec![ids[0].clone()];
        let (moved, rejected) = doc.rewire_sources(&first, &d, "out");
        assert_eq!((moved, rejected), (1, 0));
        let from_a = doc
            .graph()
            .edges
            .iter()
            .filter(|edge| edge.from == a)
            .count();
        let from_d = doc
            .graph()
            .edges
            .iter()
            .filter(|edge| edge.from == d)
            .count();
        assert_eq!((from_a, from_d), (1, 1));
    }

    #[test]
    fn inserting_an_existing_node_into_a_wire_is_one_undo_step() {
        let (mut doc, a, b) = doc_with_chain();
        let mid = doc.add_node("sort_values", Point::new(160.0, 0.0)).unwrap();
        doc.set_param(&mid, "by", json!("amount"));
        let edge = doc.graph().edges[0].id.clone();
        doc.insert_node_on_edge(&edge, &mid).unwrap();
        let edges = doc.graph().edges.clone();
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|e| e.from == a && e.to == mid));
        assert!(edges.iter().any(|e| e.from == mid && e.to == b));
        assert_eq!(doc.graph().node(&mid).unwrap().param_str("by"), "amount");
        assert!(doc.undo());
        assert_eq!(doc.graph().edges.len(), 1);
        assert_eq!(doc.graph().edges[0].from, a);
        assert!(doc.graph().node(&mid).is_some());
    }

    #[test]
    fn inserting_a_connected_node_into_a_wire_is_refused() {
        let (mut doc, _a, b, _, _) = doc_with_fan();
        let edge = doc.graph().edges[0].id.clone();
        // `b` already has a wire, so it must not be threaded through another.
        assert_eq!(doc.insert_node_on_edge(&edge, &b), Err(Reject::Type));
        assert_eq!(doc.graph().edges.len(), 2);
    }

    #[test]
    fn features_producing_lists_writers_of_a_type() {
        let doc = Document::new();
        let tables = doc.features_producing(TypeName::Dataframe);
        assert!(tables.iter().any(|def| def.id == "read_csv"));
        assert!(tables.iter().any(|def| def.id == "head"));
        assert!(tables.iter().all(|def| {
            def.output == Some(TypeName::Dataframe)
                || def.possible_outputs.contains(&TypeName::Dataframe)
        }));
        let columns = doc.features_producing(TypeName::Series);
        assert!(columns.iter().any(|def| def.id == "column"));
        assert!(!columns.iter().any(|def| def.id == "read_csv"));
    }

    #[test]
    fn clear_reroutes_keeps_the_wire() {
        let (mut doc, _, _) = doc_with_chain();
        let edge = doc.graph().edges[0].id.clone();
        doc.add_reroute(&edge, 0, Point::new(160.0, 100.0));
        doc.add_reroute(&edge, 1, Point::new(200.0, 140.0));
        doc.clear_reroutes(&edge);
        assert!(doc.graph().edges[0].reroutes.is_empty());
        assert_eq!(doc.graph().edges.len(), 1);
        assert!(doc.undo());
        assert_eq!(doc.graph().edges[0].reroutes.len(), 2);
    }

    /// One undo must restore the whole graph exactly; redo must re-apply it.
    fn assert_one_undo_round_trip(doc: &mut Document, before: &Graph, after: &Graph) {
        assert_eq!(doc.graph(), after, "the edit did not apply as expected");
        assert!(doc.undo());
        assert_eq!(doc.graph(), before, "one undo must restore the prior graph");
        assert!(doc.redo());
        assert_eq!(doc.graph(), after, "redo must re-apply the edit");
    }

    fn undo_restores_after<F: FnOnce(&mut Document)>(body: F) {
        let mut doc = Document::new();
        doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        doc.add_node("head", Point::new(300.0, 0.0)).unwrap();
        let before = doc.graph().clone();
        body(&mut doc);
        let after = doc.graph().clone();
        assert_one_undo_round_trip(&mut doc, &before, &after);
    }

    #[test]
    fn adding_a_node_is_one_undo_step() {
        undo_restores_after(|doc| {
            doc.add_node("head", Point::new(320.0, 64.0)).unwrap();
        });
    }

    #[test]
    fn moving_nodes_is_one_undo_step() {
        undo_restores_after(|doc| {
            let ids: Vec<String> = doc.graph().nodes.iter().map(|n| n.id.clone()).collect();
            doc.move_nodes(&ids, Point::new(32.0, -16.0), false);
        });
    }

    #[test]
    fn connecting_is_one_undo_step() {
        undo_restores_after(|doc| {
            let a = doc.graph().nodes[0].id.clone();
            let b = doc.graph().nodes[1].id.clone();
            doc.connect(&a, "out", &b, "in").unwrap();
        });
    }

    #[test]
    fn detaching_is_one_undo_step() {
        let (mut doc, _a, _b) = doc_with_chain();
        let edge = doc.graph().edges[0].id.clone();
        let before = doc.graph().clone();
        doc.detach(&edge);
        let after = doc.graph().clone();
        assert_one_undo_round_trip(&mut doc, &before, &after);
    }

    #[test]
    fn setting_a_param_is_one_undo_step() {
        undo_restores_after(|doc| {
            let id = doc.graph().nodes[0].id.clone();
            doc.set_param(&id, "path", json!("orders.csv"));
        });
    }

    #[test]
    fn setting_a_label_is_one_undo_step() {
        undo_restores_after(|doc| {
            let id = doc.graph().nodes[0].id.clone();
            doc.set_label(&id, "Orders".into());
        });
    }

    #[test]
    fn setting_notes_is_one_undo_step() {
        undo_restores_after(|doc| {
            let id = doc.graph().nodes[0].id.clone();
            doc.set_notes(&id, "Reads the raw file".into());
        });
    }

    #[test]
    fn adding_a_mark_is_one_undo_step() {
        undo_restores_after(|doc| {
            doc.add_mark(MarkKind::Label, Point::new(48.0, 48.0), Some([160.0, 32.0]))
                .unwrap();
        });
    }

    #[test]
    fn placing_many_is_one_undo_step() {
        undo_restores_after(|doc| {
            let positions: BTreeMap<String, Point> = doc
                .graph()
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    (
                        node.id.clone(),
                        Point::new(64.0 + index as f32 * 320.0, 48.0),
                    )
                })
                .collect();
            doc.place_many(&positions);
        });
    }

    #[test]
    fn setting_a_variable_is_one_undo_step() {
        undo_restores_after(|doc| {
            doc.set_variable(Variable {
                name: "city".into(),
                kind: VarKind::String,
                value: json!("Paris"),
            });
        });
    }
}
