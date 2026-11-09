//! Pointer gestures for the canvas. Rules stay in `graph`; this file only
//! decides which rule a gesture is asking for.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Pixels, Point as GPoint, Render, SharedString, Styled as _,
    Task, Window, canvas, div, point, px,
};
use graph::{
    CARD_WIDTH, Camera, Document, Edge, MarkKind, Node, Point, Reject, SNAP_RADIUS_PX,
    SnapCandidate, TypeName, pick_snap, reject_message, try_connect, type_pair_label,
};

use crate::layout::{self, Side, card_height, port_slots};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Hand,
    Rectangle,
    Circle,
    Label,
    Note,
    Button,
}

impl Tool {
    pub fn is_mark(self) -> bool {
        matches!(
            self,
            Self::Rectangle | Self::Circle | Self::Label | Self::Note | Self::Button
        )
    }
}

pub enum CanvasEvent {
    Edited,
    SelectionChanged,
    FocusInspector,
    OpenCatalog { filter: Option<TypeName> },
}

struct Settle {
    /// Canvas-pixel offset from the committed position, at the start of the settle.
    offsets: BTreeMap<String, Point>,
    started: Instant,
    duration: Duration,
}

#[derive(Clone)]
enum Drag {
    Move {
        origin_local: GPoint<Pixels>,
        nodes: BTreeMap<String, Point>,
        marks: BTreeMap<String, Point>,
        #[allow(dead_code)]
        duplicated: bool,
    },
    Link {
        from: String,
        from_port: String,
        detached: Option<Edge>,
        pointer: GPoint<Pixels>,
    },
    Marquee {
        start: GPoint<Pixels>,
        current: GPoint<Pixels>,
        additive: bool,
    },
    Mark {
        kind: MarkKind,
        start: Point,
        current: Point,
    },
    Pan {
        last: GPoint<Pixels>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Muted,
    Success,
    Warning,
    Danger,
    Running,
}

pub struct CanvasModel {
    focus: FocusHandle,
    editor: Document,
    dots: BTreeMap<String, Dot>,
    selection: Vec<String>,
    tool: Tool,
    space_hand: bool,
    drag: Option<Drag>,
    settle: Option<Settle>,
    reduced_motion: bool,
    status: String,
    viewport: Rc<RefCell<Option<Bounds<Pixels>>>>,
    _anim: Option<Task<()>>,
}

impl CanvasModel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            editor: Document::new(),
            dots: BTreeMap::new(),
            selection: Vec::new(),
            tool: Tool::Select,
            space_hand: false,
            drag: None,
            settle: None,
            reduced_motion: false,
            status: String::new(),
            viewport: Rc::new(RefCell::new(None)),
            _anim: None,
        }
        .with_focus(window)
    }

    fn with_focus(self, _window: &mut Window) -> Self {
        self
    }

    pub fn editor(&self) -> &Document {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Document {
        &mut self.editor
    }

    pub fn begin_edit(&mut self) {
        self.editor.begin_gesture();
    }

    pub fn commit_edit(&mut self, cx: &mut Context<Self>) {
        self.editor.commit_gesture();
        self.changed(cx);
    }

    pub fn set_param(
        &mut self,
        node: &str,
        name: &str,
        value: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        if self.editor.set_param(node, name, value) {
            self.changed(cx);
        }
    }

    pub fn set_label(&mut self, node: &str, label: String, cx: &mut Context<Self>) {
        if self.editor.set_label(node, label) {
            self.changed(cx);
        }
    }

    pub fn drop_paths(&mut self, paths: &[impl AsRef<Path>], cx: &mut Context<Self>) {
        for path in paths {
            let path = path.as_ref();
            let feature = match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
                "csv" | "tsv" | "txt" => "read_csv",
                "xlsx" | "xls" => "read_excel",
                "sqlite" | "db" | "sqlite3" => "read_sql",
                _ => continue,
            };
            let at = Point::new(48.0 + self.editor.graph().nodes.len() as f32 * 16.0, 64.0);
            let Some(id) = self.editor.add_node(feature, at) else {
                continue;
            };
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| feature.to_string());
            self.editor.set_label(&id, name);
            self.editor
                .set_param(&id, "path", serde_json::json!(path.display().to_string()));
            self.selection = vec![id];
        }
        self.changed(cx);
        cx.emit(CanvasEvent::SelectionChanged);
        cx.emit(CanvasEvent::FocusInspector);
    }

    pub fn selection(&self) -> &[String] {
        &self.selection
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn status_line(&self) -> &str {
        &self.status
    }

    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tool = tool;
        cx.notify();
    }

    pub fn set_dots(&mut self, dots: BTreeMap<String, Dot>, cx: &mut Context<Self>) {
        let running = dots.values().any(|dot| *dot == Dot::Running);
        self.dots = dots;
        if running && !self.reduced_motion {
            self.kick_animation(cx);
        }
        cx.notify();
    }

    pub fn set_reduced_motion(&mut self, reduced: bool, cx: &mut Context<Self>) {
        self.reduced_motion = reduced;
        if reduced {
            self.settle = None;
        }
        cx.notify();
    }

    pub fn is_reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub fn load_document(&mut self, editor: Document, cx: &mut Context<Self>) {
        self.editor = editor;
        self.selection.clear();
        self.drag = None;
        self.settle = None;
        cx.emit(CanvasEvent::Edited);
        cx.emit(CanvasEvent::SelectionChanged);
        cx.notify();
    }

    pub fn replace_editor(&mut self, editor: Document) {
        self.editor = editor;
    }

    pub fn select_only(&mut self, id: impl Into<String>, cx: &mut Context<Self>) {
        self.selection = vec![id.into()];
        cx.emit(CanvasEvent::SelectionChanged);
        cx.notify();
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.selection
            .iter()
            .find_map(|id| self.editor.graph().node(id))
    }

    pub fn add_feature(&mut self, feature: &str, cx: &mut Context<Self>) -> Option<String> {
        let origin = self.local_origin();
        let at = self
            .editor
            .graph()
            .camera
            .to_canvas(Point::new(origin.0, origin.1));
        let id = if let Some(selected) = self.selected_node().map(|node| node.id.clone()) {
            self.editor.add_after(&selected, feature)
        } else {
            self.editor.add_node(feature, at)
        }?;
        self.selection = vec![id.clone()];
        self.changed(cx);
        cx.emit(CanvasEvent::FocusInspector);
        cx.emit(CanvasEvent::SelectionChanged);
        Some(id)
    }

    pub fn delete_selection(&mut self, cx: &mut Context<Self>) {
        let (nodes, marks) = self.split_selection();
        if nodes.is_empty() && marks.is_empty() {
            return;
        }
        self.editor.delete(&nodes, &marks);
        self.selection.clear();
        self.changed(cx);
        cx.emit(CanvasEvent::SelectionChanged);
    }

    pub fn duplicate_selection(&mut self, cx: &mut Context<Self>) {
        let (nodes, marks) = self.split_selection();
        let (new_nodes, new_marks) = self.editor.duplicate(&nodes, &marks);
        self.selection = new_nodes.into_iter().chain(new_marks).collect();
        self.changed(cx);
        cx.emit(CanvasEvent::SelectionChanged);
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.editor.undo() {
            self.selection.retain(|id| {
                self.editor.graph().node(id).is_some() || self.editor.graph().mark(id).is_some()
            });
            self.changed(cx);
            cx.emit(CanvasEvent::SelectionChanged);
        }
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.editor.redo() {
            self.changed(cx);
            cx.emit(CanvasEvent::SelectionChanged);
        }
    }

    pub fn fit(&mut self, cx: &mut Context<Self>) {
        let bounds = self
            .viewport
            .borrow()
            .map(|bounds| (bounds.size.width.as_f32(), bounds.size.height.as_f32()));
        let Some((w, h)) = bounds else {
            return;
        };
        let ids: Vec<String> = if self.selection.is_empty() {
            self.editor
                .graph()
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .chain(self.editor.graph().marks.iter().map(|mark| mark.id.clone()))
                .collect()
        } else {
            self.selection.clone()
        };
        let Some(rect) = graph::bounds_of(self.editor.graph(), &ids, card_height) else {
            return;
        };
        self.editor.set_camera(graph::fit_camera(w, h, rect, 32.0));
        cx.emit(CanvasEvent::Edited);
        cx.notify();
    }

    pub fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let center = self
            .viewport
            .borrow()
            .map(|bounds| point(bounds.size.width * 0.5, bounds.size.height * 0.5))
            .unwrap_or_else(|| point(px(200.0), px(200.0)));
        let camera = graph::zoom_toward(
            self.editor.graph().camera,
            Point::new(center.x.as_f32(), center.y.as_f32()),
            factor,
        );
        self.editor.set_camera(camera);
        cx.emit(CanvasEvent::Edited);
        cx.notify();
    }

    /// Seed a link drag so a screenshot can show snap or reject without a pointer.
    pub fn seed_link_drag(
        &mut self,
        from: &str,
        from_port: &str,
        local: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(Drag::Link {
            from: from.to_string(),
            from_port: from_port.to_string(),
            detached: None,
            pointer: point(px(local.0), px(local.1)),
        });
        self.refresh_link_status();
        cx.notify();
    }

    pub fn set_space_hand(&mut self, down: bool, cx: &mut Context<Self>) {
        self.space_hand = down;
        cx.notify();
    }

    fn split_selection(&self) -> (Vec<String>, Vec<String>) {
        let mut nodes = Vec::new();
        let mut marks = Vec::new();
        for id in &self.selection {
            if self.editor.graph().node(id).is_some() {
                nodes.push(id.clone());
            } else if self.editor.graph().mark(id).is_some() {
                marks.push(id.clone());
            }
        }
        (nodes, marks)
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(CanvasEvent::Edited);
        cx.notify();
    }

    fn local_origin(&self) -> (f32, f32) {
        (48.0, 48.0)
    }

    fn local_of(&self, window_point: GPoint<Pixels>) -> GPoint<Pixels> {
        let origin = self
            .viewport
            .borrow()
            .map(|bounds| bounds.origin)
            .unwrap_or_else(|| point(px(0.0), px(0.0)));
        window_point - origin
    }

    fn canvas_of(&self, local: GPoint<Pixels>) -> Point {
        self.editor
            .graph()
            .camera
            .to_canvas(Point::new(local.x.as_f32(), local.y.as_f32()))
    }

    fn hand(&self) -> bool {
        self.tool == Tool::Hand || self.space_hand
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_readonly() && event.button == MouseButton::Left && self.tool.is_mark() {
            return;
        }
        let local = self.local_of(event.position);
        window.focus(&self.focus, cx);
        if event.button == MouseButton::Middle || (event.button == MouseButton::Left && self.hand())
        {
            self.drag = Some(Drag::Pan { last: local });
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        if self.tool.is_mark() {
            let canvas = self.canvas_of(local);
            self.editor.begin_gesture();
            // The gesture stays open so a cancelled drag (escape) can drop it.
            // The mark itself is created on release, after this gesture is cancelled.
            self.drag = Some(Drag::Mark {
                kind: mark_kind(self.tool),
                start: canvas,
                current: canvas,
            });
            cx.notify();
            return;
        }
        self.drag = Some(Drag::Marquee {
            start: local,
            current: local,
            additive: event.modifiers.shift,
        });
        cx.notify();
    }

    fn port_down(
        &mut self,
        node: &str,
        port: &str,
        side: Side,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.editor.is_readonly() {
            return;
        }
        let local = self.local_of(event.position);
        window.focus(&self.focus, cx);
        if side == Side::Out {
            self.editor.begin_gesture();
            self.drag = Some(Drag::Link {
                from: node.to_string(),
                from_port: port.to_string(),
                detached: None,
                pointer: local,
            });
        } else if let Some(edge) = self
            .editor
            .graph()
            .edges
            .iter()
            .find(|edge| edge.to == node && edge.to_port == port)
            .cloned()
        {
            self.editor.begin_gesture();
            let detached = self.editor.detach(&edge.id);
            if let Some(detached) = detached.as_ref() {
                self.drag = Some(Drag::Link {
                    from: detached.from.clone(),
                    from_port: detached.from_port.clone(),
                    detached: Some(detached.clone()),
                    pointer: local,
                });
            }
        }
        self.refresh_link_status();
        cx.notify();
    }

    fn card_down(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        window.focus(&self.focus, cx);
        if event.click_count >= 2 {
            if !self.selection.iter().any(|item| item == id) {
                self.selection = vec![id.to_string()];
                cx.emit(CanvasEvent::SelectionChanged);
            }
            cx.emit(CanvasEvent::FocusInspector);
            cx.notify();
            return;
        }
        let local = self.local_of(event.position);
        if event.modifiers.shift {
            if self.selection.iter().any(|item| item == id) {
                self.selection.retain(|item| item != id);
            } else {
                self.selection.push(id.to_string());
            }
        } else if !self.selection.iter().any(|item| item == id) {
            self.selection = vec![id.to_string()];
        }
        cx.emit(CanvasEvent::SelectionChanged);
        if self.editor.is_readonly() {
            cx.notify();
            return;
        }
        let mut nodes = BTreeMap::new();
        let mut marks = BTreeMap::new();
        if event.modifiers.alt {
            let (node_ids, mark_ids) = self.split_selection();
            let (new_nodes, new_marks) = self.editor.duplicate(&node_ids, &mark_ids);
            self.selection = new_nodes
                .iter()
                .cloned()
                .chain(new_marks.iter().cloned())
                .collect();
            for id in &self.selection {
                if let Some(node) = self.editor.graph().node(id) {
                    nodes.insert(id.clone(), node.position_point());
                } else if let Some(mark) = self.editor.graph().mark(id) {
                    marks.insert(id.clone(), Point::new(mark.position[0], mark.position[1]));
                }
            }
        } else {
            for id in &self.selection {
                if let Some(node) = self.editor.graph().node(id) {
                    nodes.insert(id.clone(), node.position_point());
                } else if let Some(mark) = self.editor.graph().mark(id) {
                    marks.insert(id.clone(), Point::new(mark.position[0], mark.position[1]));
                }
            }
        }
        self.editor.begin_gesture();
        self.settle = None;
        self.drag = Some(Drag::Move {
            origin_local: local,
            nodes,
            marks,
            duplicated: event.modifiers.alt,
        });
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let local = self.local_of(event.position);
        let Some(mut drag) = self.drag.clone() else {
            return;
        };
        let mut refresh_link = false;
        match &mut drag {
            Drag::Pan { last } => {
                let dx = local.x.as_f32() - last.x.as_f32();
                let dy = local.y.as_f32() - last.y.as_f32();
                let camera = self.editor.graph().camera;
                self.editor.set_camera(Camera {
                    x: camera.x + dx,
                    y: camera.y + dy,
                    zoom: camera.zoom,
                });
                *last = local;
                cx.emit(CanvasEvent::Edited);
            }
            Drag::Move {
                origin_local,
                nodes,
                marks,
                ..
            } => {
                let zoom = self.editor.graph().camera.zoom.max(0.05);
                let dx = (local.x.as_f32() - origin_local.x.as_f32()) / zoom;
                let dy = (local.y.as_f32() - origin_local.y.as_f32()) / zoom;
                let placed_nodes = nodes
                    .iter()
                    .map(|(id, origin)| (id.clone(), Point::new(origin.x + dx, origin.y + dy)))
                    .collect();
                let placed_marks = marks
                    .iter()
                    .map(|(id, origin)| (id.clone(), Point::new(origin.x + dx, origin.y + dy)))
                    .collect();
                self.editor.place(&placed_nodes, &placed_marks, false);
            }
            Drag::Link { pointer, .. } => {
                *pointer = local;
                refresh_link = true;
            }
            Drag::Marquee { current, .. } => {
                *current = local;
            }
            Drag::Mark { current, .. } => {
                *current = self.canvas_of(local);
            }
        }
        self.drag = Some(drag);
        if refresh_link {
            self.refresh_link_status();
        }
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let local = self.local_of(event.position);
        let drag = self.drag.take();
        match drag {
            Some(Drag::Pan { .. }) => {}
            Some(Drag::Move {
                origin_local,
                nodes,
                marks,
                ..
            }) => {
                let zoom = self.editor.graph().camera.zoom.max(0.05);
                let dx = (local.x.as_f32() - origin_local.x.as_f32()) / zoom;
                let dy = (local.y.as_f32() - origin_local.y.as_f32()) / zoom;
                let snap = !event.modifiers.alt;
                let mut placed_nodes = BTreeMap::new();
                let mut offsets = BTreeMap::new();
                for (id, origin) in &nodes {
                    let raw = Point::new(origin.x + dx, origin.y + dy);
                    let next = if snap { graph::snap_grid(raw) } else { raw };
                    placed_nodes.insert(id.clone(), next);
                    offsets.insert(id.clone(), Point::new(raw.x - next.x, raw.y - next.y));
                }
                let placed_marks = marks
                    .iter()
                    .map(|(id, origin)| {
                        let raw = Point::new(origin.x + dx, origin.y + dy);
                        let next = if snap { graph::snap_grid(raw) } else { raw };
                        (id.clone(), next)
                    })
                    .collect();
                self.editor.place(&placed_nodes, &placed_marks, false);
                self.editor.commit_gesture();
                if !self.reduced_motion
                    && offsets
                        .values()
                        .any(|offset| offset.x.abs() + offset.y.abs() > 0.5)
                {
                    self.settle = Some(Settle {
                        offsets,
                        started: Instant::now(),
                        duration: Duration::from_millis(160),
                    });
                    self.kick_animation(cx);
                }
                self.changed(cx);
            }
            Some(Drag::Link {
                from,
                from_port,
                detached,
                ..
            }) => {
                let snap = self.snap_at(local, &from, &from_port);
                if let Some((to, to_port)) = snap {
                    let _ = self.editor.connect(&from, &from_port, &to, &to_port);
                    self.status.clear();
                } else if detached.is_some() {
                    self.status = "wire removed".to_string();
                } else {
                    self.status.clear();
                    let filter = self.editor.graph().node(&from).and_then(|_| {
                        graph::output_types(self.editor.graph(), &from)
                            .into_iter()
                            .next()
                    });
                    if snap.is_none() && detached.is_none() {
                        cx.emit(CanvasEvent::OpenCatalog { filter });
                    }
                }
                self.editor.commit_gesture();
                self.changed(cx);
            }
            Some(Drag::Marquee {
                start,
                current,
                additive,
            }) => {
                let rect = rect_of(start, current);
                if (current.x - start.x).abs() < px(3.0) && (current.y - start.y).abs() < px(3.0) {
                    if !additive {
                        self.selection.clear();
                        cx.emit(CanvasEvent::SelectionChanged);
                    }
                } else {
                    let mut hits = Vec::new();
                    for node in &self.editor.graph().nodes {
                        let origin = node.position_point();
                        let window = self.editor.graph().camera.to_window(origin);
                        let zoom = self.editor.graph().camera.zoom;
                        let card = graph::Rect {
                            x: window.x,
                            y: window.y,
                            w: CARD_WIDTH * zoom,
                            h: card_height(node) * zoom,
                        };
                        if rects_overlap(rect, card) {
                            hits.push(node.id.clone());
                        }
                    }
                    if additive {
                        for id in hits {
                            if !self.selection.contains(&id) {
                                self.selection.push(id);
                            }
                        }
                    } else {
                        self.selection = hits;
                    }
                    cx.emit(CanvasEvent::SelectionChanged);
                }
                cx.notify();
            }
            Some(Drag::Mark {
                kind,
                start,
                current,
            }) => {
                self.editor.cancel_gesture();
                let x = start.x.min(current.x);
                let y = start.y.min(current.y);
                let w = (start.x - current.x).abs().max(16.0);
                let h = (start.y - current.y).abs().max(16.0);
                if let Some(id) = self.editor.add_mark(kind, Point::new(x, y), Some([w, h])) {
                    self.selection = vec![id];
                    cx.emit(CanvasEvent::SelectionChanged);
                }
                self.changed(cx);
            }
            None => {}
        }
    }

    fn snap_at(
        &self,
        local: GPoint<Pixels>,
        from: &str,
        from_port: &str,
    ) -> Option<(String, String)> {
        let pointer = Point::new(local.x.as_f32(), local.y.as_f32());
        let mut candidates = Vec::new();
        for node in &self.editor.graph().nodes {
            if node.id == from {
                continue;
            }
            for slot in port_slots(node) {
                if slot.side != Side::In {
                    continue;
                }
                let window = self.editor.graph().camera.to_window(slot.center);
                let dx = window.x - pointer.x;
                let dy = window.y - pointer.y;
                let distance = (dx * dx + dy * dy).sqrt();
                let compatible =
                    try_connect(self.editor.graph(), from, from_port, &node.id, &slot.name).is_ok();
                candidates.push(SnapCandidate {
                    node: node.id.clone(),
                    port: slot.name,
                    distance_px: distance,
                    compatible,
                });
            }
        }
        let index = pick_snap(&candidates)?;
        Some((
            candidates[index].node.clone(),
            candidates[index].port.clone(),
        ))
    }

    fn refresh_link_status(&mut self) {
        let Some(Drag::Link {
            from,
            from_port,
            pointer,
            ..
        }) = &self.drag
        else {
            return;
        };
        let from = from.clone();
        let from_port = from_port.clone();
        let pointer = *pointer;
        // Temporarily clear so snap_at does not recurse on drag_from_port... it reads drag.
        let snap = self.snap_at(pointer, &from, &from_port);
        if let Some((to, to_port)) = snap {
            let (left, right) = type_pair_label(self.editor.graph(), &from, &to, &to_port);
            self.status = format!("{left} → {right}");
            return;
        }
        if let Some((node, port, reason)) = self.nearest_reject(pointer, &from, &from_port) {
            let (left, right) = type_pair_label(self.editor.graph(), &from, &node, &port);
            self.status = reject_message(reason, &left, &right);
        } else {
            self.status.clear();
        }
    }

    fn nearest_reject(
        &self,
        local: GPoint<Pixels>,
        from: &str,
        from_port: &str,
    ) -> Option<(String, String, Reject)> {
        let pointer = Point::new(local.x.as_f32(), local.y.as_f32());
        let mut best: Option<(f32, String, String, Reject)> = None;
        for node in &self.editor.graph().nodes {
            if node.id == from {
                continue;
            }
            for slot in port_slots(node) {
                if slot.side != Side::In {
                    continue;
                }
                let window = self.editor.graph().camera.to_window(slot.center);
                let distance =
                    ((window.x - pointer.x).powi(2) + (window.y - pointer.y).powi(2)).sqrt();
                if distance > SNAP_RADIUS_PX {
                    continue;
                }
                if let Err(reason) =
                    try_connect(self.editor.graph(), from, from_port, &node.id, &slot.name)
                    && best
                        .as_ref()
                        .map(|(best, _, _, _)| distance < *best)
                        .unwrap_or(true)
                {
                    best = Some((distance, node.id.clone(), slot.name.clone(), reason));
                }
            }
        }
        best.map(|(_, node, port, reason)| (node, port, reason))
    }

    fn kick_animation(&mut self, cx: &mut Context<Self>) {
        if self._anim.is_some() {
            return;
        }
        self._anim = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        let finished = this.tick_settle();
                        cx.notify();
                        finished
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            let _ = this.update(cx, |this, _| {
                this._anim = None;
            });
        }));
    }

    fn tick_settle(&mut self) -> bool {
        let settle_done = match &self.settle {
            Some(settle) if settle.started.elapsed() >= settle.duration => {
                self.settle = None;
                true
            }
            Some(_) => false,
            None => true,
        };
        let running = self.dots.values().any(|dot| *dot == Dot::Running) && !self.reduced_motion;
        settle_done && !running
    }

    fn settle_offset(&self, id: &str) -> Point {
        let Some(settle) = &self.settle else {
            return Point::new(0.0, 0.0);
        };
        let Some(offset) = settle.offsets.get(id) else {
            return Point::new(0.0, 0.0);
        };
        let t = (settle.started.elapsed().as_secs_f32() / settle.duration.as_secs_f32())
            .clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - t).powi(3);
        Point::new(offset.x * (1.0 - ease), offset.y * (1.0 - ease))
    }

    fn link_visual(&self) -> Option<LinkVisual> {
        let Drag::Link {
            from,
            from_port,
            pointer,
            ..
        } = self.drag.as_ref()?
        else {
            return None;
        };
        let source = self.editor.graph().node(from)?;
        let slot = port_slots(source)
            .into_iter()
            .find(|slot| slot.name == *from_port && slot.side == Side::Out)?;
        let snap = self.snap_at(*pointer, from, from_port);
        let reject = if snap.is_none() {
            self.nearest_reject(*pointer, from, from_port)
        } else {
            None
        };
        let end = if let Some((to, to_port)) = &snap {
            self.editor
                .graph()
                .node(to)
                .and_then(|node| {
                    port_slots(node)
                        .into_iter()
                        .find(|slot| slot.name == *to_port && slot.side == Side::In)
                        .map(|slot| slot.center)
                })
                .unwrap_or_else(|| self.canvas_of(*pointer))
        } else {
            self.canvas_of(*pointer)
        };
        Some(LinkVisual {
            from: slot.center,
            to: end,
            snapped: snap,
            reject: reject.map(|(_, _, reason)| reason),
        })
    }
}

struct LinkVisual {
    from: Point,
    to: Point,
    snapped: Option<(String, String)>,
    reject: Option<Reject>,
}

impl EventEmitter<CanvasEvent> for CanvasModel {}

impl Focusable for CanvasModel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CanvasModel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = self.viewport.clone();
        let theme_border = cx.theme().border;
        let theme_muted = cx.theme().muted_foreground;
        let theme_primary = cx.theme().primary;
        let theme_danger = cx.theme().danger;
        let theme_bg = cx.theme().background;
        let theme_fg = cx.theme().foreground;
        let theme_card = cx.theme().secondary;
        let theme_success = cx.theme().success;
        let theme_warning = cx.theme().warning;
        let radius = cx.theme().radius;
        let mono = cx.theme().mono_font_family.clone();

        let camera = self.editor.graph().camera;
        let nodes = self.editor.graph().nodes.clone();
        let edges = self.editor.graph().edges.clone();
        let marks = self.editor.graph().marks.clone();
        let selection = self.selection.clone();
        let link = self.link_visual();
        let marquee = match &self.drag {
            Some(Drag::Marquee { start, current, .. }) => Some((*start, *current)),
            _ => None,
        };
        let settle_ids: Vec<(String, Point)> = nodes
            .iter()
            .map(|node| (node.id.clone(), self.settle_offset(&node.id)))
            .collect();

        let entity = cx.entity();
        div()
            .id("node-canvas")
            .key_context("Canvas")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme_bg)
            .cursor(if self.hand() {
                CursorStyle::OpenHand
            } else if link.as_ref().and_then(|item| item.reject).is_some() {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::Arrow
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| {
                    this.mouse_down(event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event, window, cx| {
                    this.mouse_down(event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event, _, cx| {
                this.mouse_move(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| {
                    this.mouse_up(event, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, event, _, cx| {
                    this.mouse_up(event, cx);
                }),
            )
            .on_drop({
                let entity = cx.entity();
                move |paths: &gpui_kit::ExternalPaths, _window, cx| {
                    entity.update(cx, |this, cx| this.drop_paths(&paths.0, cx));
                }
            })
            .on_scroll(cx.listener(|this, event: &gpui_kit::ScrollEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.0));
                if event.modifiers.control || event.modifiers.platform {
                    let factor = if delta.y.as_f32() > 0.0 {
                        1.08
                    } else {
                        1.0 / 1.08
                    };
                    let local = this.local_of(event.position);
                    let camera = graph::zoom_toward(
                        this.editor.graph().camera,
                        Point::new(local.x.as_f32(), local.y.as_f32()),
                        factor,
                    );
                    this.editor.set_camera(camera);
                    cx.emit(CanvasEvent::Edited);
                    cx.notify();
                } else {
                    let camera = this.editor.graph().camera;
                    this.editor.set_camera(Camera {
                        x: camera.x - delta.x.as_f32(),
                        y: camera.y - delta.y.as_f32(),
                        zoom: camera.zoom,
                    });
                    cx.emit(CanvasEvent::Edited);
                    cx.notify();
                }
            }))
            .when(!self.status.is_empty(), |this| {
                let text = self.status.clone();
                this.child(
                    div()
                        .absolute()
                        .bottom(px(8.0))
                        .left(px(12.0))
                        .text_xs()
                        .text_color(if link.as_ref().and_then(|item| item.reject).is_some() {
                            theme_danger
                        } else {
                            theme_muted
                        })
                        .child(text),
                )
            })
            .child({
                let painted_selection = selection.clone();
                canvas(
                    move |bounds, _, _| {
                        *viewport.borrow_mut() = Some(bounds);
                    },
                    move |bounds, _, window, cx| {
                        paint_scene(
                            bounds,
                            camera,
                            &nodes,
                            &edges,
                            &marks,
                            &painted_selection,
                            &settle_ids,
                            link.as_ref(),
                            theme_muted,
                            theme_primary,
                            theme_danger,
                            window,
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full()
            })
            .children(self.editor.graph().nodes.iter().map(|node| {
                card_element(
                    entity.clone(),
                    node,
                    self.dots.get(&node.id).copied().unwrap_or(if node.unknown {
                        Dot::Danger
                    } else {
                        Dot::Muted
                    }),
                    self.reduced_motion,
                    camera,
                    self.settle_offset(&node.id),
                    selection.iter().any(|id| id == &node.id),
                    theme_card,
                    theme_fg,
                    theme_muted,
                    theme_primary,
                    theme_border,
                    theme_success,
                    theme_warning,
                    theme_danger,
                    radius,
                    &mono,
                )
            }))
            .children(
                if self.editor.graph().nodes.is_empty() && self.editor.graph().marks.is_empty() {
                    Some(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(theme_muted)
                                    .child("Drop a CSV, XLSX, or SQLite file"),
                            ),
                    )
                } else {
                    None
                },
            )
            .when_some(marquee, |this, (start, current)| {
                let x = start.x.min(current.x);
                let y = start.y.min(current.y);
                let w = (start.x - current.x).abs();
                let h = (start.y - current.y).abs();
                this.child(
                    div()
                        .absolute()
                        .left(x)
                        .top(y)
                        .w(w)
                        .h(h)
                        .border_1()
                        .border_color(theme_primary)
                        .bg(theme_primary.opacity(0.08)),
                )
            })
    }
}

fn mark_kind(tool: Tool) -> MarkKind {
    match tool {
        Tool::Circle => MarkKind::Circle,
        Tool::Label => MarkKind::Label,
        Tool::Note => MarkKind::Note,
        Tool::Button => MarkKind::Button,
        _ => MarkKind::Rectangle,
    }
}

fn rect_of(a: GPoint<Pixels>, b: GPoint<Pixels>) -> graph::Rect {
    graph::Rect {
        x: a.x.min(b.x).as_f32(),
        y: a.y.min(b.y).as_f32(),
        w: (a.x - b.x).abs().as_f32(),
        h: (a.y - b.y).abs().as_f32(),
    }
}

fn rects_overlap(a: graph::Rect, b: graph::Rect) -> bool {
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

#[allow(clippy::too_many_arguments)]
fn paint_scene(
    bounds: Bounds<Pixels>,
    camera: Camera,
    nodes: &[Node],
    edges: &[Edge],
    marks: &[graph::Mark],
    selection: &[String],
    _settle: &[(String, Point)],
    link: Option<&LinkVisual>,
    muted: impl Into<gpui_kit::Hsla>,
    primary: impl Into<gpui_kit::Hsla>,
    danger: impl Into<gpui_kit::Hsla>,
    window: &mut Window,
    _cx: &mut App,
) {
    let muted = muted.into();
    let primary = primary.into();
    let danger = danger.into();
    let origin = bounds.origin;
    // Grid dots in the visible area.
    let zoom = camera.zoom;
    if zoom > 0.3 {
        let step = graph::GRID_PX * zoom;
        let mut path = gpui_kit::PathBuilder::fill();
        let mut x = camera.x % step;
        if x < 0.0 {
            x += step;
        }
        while x < bounds.size.width.as_f32() {
            let mut y = camera.y % step;
            if y < 0.0 {
                y += step;
            }
            while y < bounds.size.height.as_f32() {
                let center = origin + point(px(x), px(y));
                path.move_to(center);
                path.line_to(center + point(px(1.0), px(0.0)));
                path.line_to(center + point(px(1.0), px(1.0)));
                path.line_to(center + point(px(0.0), px(1.0)));
                y += step;
            }
            x += step;
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, muted.opacity(0.35));
        }
    }

    for mark in marks {
        if !matches!(mark.kind, MarkKind::Rectangle | MarkKind::Circle) {
            continue;
        }
        let _ = selection;
        let _ = mark;
    }

    for edge in edges {
        let Some(cubic) = layout::edge_cubic(nodes, edge) else {
            continue;
        };
        let selected = selection.iter().any(|id| id == &edge.id);
        paint_cubic(
            origin,
            camera,
            cubic,
            if selected { primary } else { muted },
            if selected { px(2.0) } else { px(1.25) },
            window,
        );
    }
    if let Some(link) = link {
        let color = if link.reject.is_some() {
            danger
        } else if link.snapped.is_some() {
            primary
        } else {
            muted
        };
        paint_cubic(
            origin,
            camera,
            layout::wire_cubic(link.from, link.to),
            color,
            px(1.75),
            window,
        );
    }
}

fn paint_cubic(
    origin: GPoint<Pixels>,
    camera: Camera,
    cubic: layout::Cubic,
    color: gpui_kit::Hsla,
    width: Pixels,
    window: &mut Window,
) {
    let map = |at: Point| {
        let window_point = camera.to_window(at);
        origin + point(px(window_point.x), px(window_point.y))
    };
    let mut path = gpui_kit::PathBuilder::stroke(width);
    path.move_to(map(cubic.from));
    path.cubic_bezier_to(map(cubic.to), map(cubic.c1), map(cubic.c2));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn card_element(
    entity: Entity<CanvasModel>,
    node: &Node,
    dot_kind: Dot,
    reduced_motion: bool,
    camera: Camera,
    offset: Point,
    selected: bool,
    card: gpui_kit::Hsla,
    fg: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    success: gpui_kit::Hsla,
    warning: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    radius: Pixels,
    mono: &str,
) -> gpui_kit::AnyElement {
    let zoom = camera.zoom;
    let origin = node.position_point();
    let window = camera.to_window(Point::new(origin.x + offset.x, origin.y + offset.y));
    let width = px(CARD_WIDTH * zoom);
    let id = node.id.clone();
    let title = node.label.clone();
    let summary = node.summary();
    let inputs = node.input_ports();
    let output = node.output_port();
    let pulse = if dot_kind == Dot::Running && !reduced_motion {
        let wave = (Instant::now().elapsed().as_secs_f32() * 5.0).sin().abs();
        0.35 + 0.65 * wave
    } else {
        1.0
    };
    let dot = match dot_kind {
        Dot::Success => success,
        Dot::Warning => warning,
        Dot::Danger => danger,
        Dot::Running => primary,
        Dot::Muted => muted,
    }
    .opacity(pulse);
    let bg = if selected {
        primary.opacity(0.12)
    } else {
        card
    };
    let border_color = if selected { primary } else { border };

    div()
        .id(SharedString::from(format!("card-{id}")))
        .absolute()
        .left(px(window.x))
        .top(px(window.y))
        .w(width)
        .flex()
        .flex_col()
        .gap_0()
        .p(px(8.0 * zoom))
        .rounded(radius)
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            let id = id.clone();
            move |event, window, cx| {
                cx.stop_propagation();
                entity.update(cx, |this, cx| this.card_down(&id, event, window, cx));
            }
        })
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(div().text_sm().text_color(fg).child(title))
                .child(div().size(px(8.0 * zoom)).rounded_full().bg(dot)),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .font_family(mono)
                .child(summary),
        )
        .child({
            let mut row = div().flex().flex_row().justify_between().mt(px(6.0 * zoom));
            let mut inputs_col = div().flex().flex_col().gap(px(2.0 * zoom));
            for port in &inputs {
                let name = port.name.clone();
                let label = format!(
                    "{} {}",
                    port.name,
                    port.accepts
                        .iter()
                        .map(|ty| ty.as_str())
                        .collect::<Vec<_>>()
                        .join("|")
                );
                let node_id = id.clone();
                let entity = entity.clone();
                inputs_col = inputs_col.child(
                    div()
                        .id(SharedString::from(format!("in-{node_id}-{name}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .h(px(layout::PORT_ROW * zoom))
                        .child(
                            div()
                                .size(px(12.0 * zoom))
                                .rounded_full()
                                .bg(primary)
                                .border_1()
                                .border_color(border),
                        )
                        .child(div().text_xs().text_color(muted).child(label))
                        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                            cx.stop_propagation();
                            entity.update(cx, |this, cx| {
                                this.port_down(&node_id, &name, Side::In, event, window, cx);
                            });
                        }),
                );
            }
            row = row.child(inputs_col);
            if let Some(port) = output {
                let name = port.name.clone();
                let label = port
                    .accepts
                    .iter()
                    .map(|ty| ty.as_str())
                    .collect::<Vec<_>>()
                    .join("|");
                let node_id = id.clone();
                let entity = entity.clone();
                row = row.child(
                    div()
                        .id(SharedString::from(format!("out-{node_id}-{name}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .h(px(layout::PORT_ROW * zoom))
                        .child(div().text_xs().text_color(muted).child(label))
                        .child(div().size(px(12.0 * zoom)).rounded_full().bg(primary))
                        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                            cx.stop_propagation();
                            entity.update(cx, |this, cx| {
                                this.port_down(&node_id, &name, Side::Out, event, window, cx);
                            });
                        }),
                );
            }
            let _ = (success, warning);
            row
        })
        .into_any_element()
}
