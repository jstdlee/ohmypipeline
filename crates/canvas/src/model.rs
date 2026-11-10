//! Pointer gestures for the canvas. Rules stay in `graph`; this file only
//! decides which rule a gesture is asking for.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Pixels, Point as GPoint, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};
use graph::{
    Camera, Document, Edge, MarkKind, Node, Point, Reject, SNAP_RADIUS_PX, SnapCandidate, TypeName,
    card_width, output_types, pick_snap, reject_message, try_connect, type_pair_label,
};

use crate::layout::{self, Side, card_height, port_slots};
use crate::motion;
use crate::plot::{feature_chart, points_from};

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

struct CameraAnim {
    from: Camera,
    to: Camera,
    started: Instant,
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
        /// Canvas point under the pointer.
        pointer: Point,
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
    Resize {
        id: String,
        origin_local: GPoint<Pixels>,
        start_size: [f32; 2],
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

#[derive(Clone, Debug)]
pub struct FrameView {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
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
    camera_anim: Option<CameraAnim>,
    frame_armed: bool,
    hovered: Option<String>,
    flow: f32,
    clip: Option<Clip>,
    label_input: Entity<gpui_kit::component::input::InputState>,
    editing: Option<String>,
    frames: BTreeMap<String, FrameView>,
    reduced_motion: bool,
    status: String,
    viewport: Rc<RefCell<Option<Bounds<Pixels>>>>,
}

impl CanvasModel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let label_input = cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx));
        cx.subscribe(
            &label_input,
            |this, input, event: &gpui_kit::component::input::InputEvent, cx| {
                if matches!(event, gpui_kit::component::input::InputEvent::Blur)
                    && this.editing.is_some()
                {
                    let text = input.read(cx).value().to_string();
                    if let Some(id) = this.editing.clone() {
                        this.editor.set_mark_text(&id, text);
                        this.editing = None;
                        this.changed(cx);
                    }
                }
            },
        )
        .detach();
        Self {
            focus: cx.focus_handle(),
            editor: Document::new(),
            dots: BTreeMap::new(),
            selection: Vec::new(),
            tool: Tool::Select,
            space_hand: false,
            drag: None,
            settle: None,
            camera_anim: None,
            frame_armed: false,
            hovered: None,
            flow: 0.0,
            clip: None,
            label_input,
            editing: None,
            frames: BTreeMap::new(),
            reduced_motion: false,
            status: String::new(),
            viewport: Rc::new(RefCell::new(None)),
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

    pub fn set_python_inputs(
        &mut self,
        node: &str,
        inputs: Vec<graph::PortRecord>,
        cx: &mut Context<Self>,
    ) {
        self.editor.set_python_inputs(node, inputs);
        self.changed(cx);
    }

    pub fn set_mark_text(&mut self, id: &str, text: String, cx: &mut Context<Self>) {
        self.editor.set_mark_text(id, text);
        self.changed(cx);
    }

    pub fn set_mark_focus(&mut self, id: &str, focus: Option<String>, cx: &mut Context<Self>) {
        self.editor.set_mark_focus(id, focus);
        self.changed(cx);
    }

    pub fn selected_mark(&self) -> Option<&graph::Mark> {
        self.selection
            .iter()
            .find_map(|id| self.editor.graph().mark(id))
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
        let _ = running;
        cx.notify();
    }

    pub fn set_reduced_motion(&mut self, reduced: bool, cx: &mut Context<Self>) {
        self.reduced_motion = reduced;
        if reduced {
            self.settle = None;
            if let Some(anim) = self.camera_anim.take() {
                self.editor.set_camera(anim.to);
            }
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
        self.camera_anim = None;
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
        self.enter_node(&id);
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

    pub fn copy_selection(&mut self) {
        let (node_ids, mark_ids) = self.split_selection();
        let nodes: Vec<_> = node_ids
            .iter()
            .filter_map(|id| self.editor.graph().node(id).cloned())
            .collect();
        let selected: std::collections::BTreeSet<_> = node_ids.iter().cloned().collect();
        let edges: Vec<_> = self
            .editor
            .graph()
            .edges
            .iter()
            .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
            .cloned()
            .collect();
        let marks: Vec<_> = mark_ids
            .iter()
            .filter_map(|id| self.editor.graph().mark(id).cloned())
            .collect();
        if !nodes.is_empty() || !marks.is_empty() {
            self.clip = Some(Clip {
                nodes,
                edges,
                marks,
            });
        }
    }

    pub fn cut_selection(&mut self, cx: &mut Context<Self>) {
        self.copy_selection();
        self.delete_selection(cx);
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(clip) = self.clip.clone() else {
            return;
        };
        let (nodes, marks) = self.editor.paste_copies(clip.nodes, clip.edges, clip.marks);
        self.selection = nodes.into_iter().chain(marks).collect();
        self.changed(cx);
        cx.emit(CanvasEvent::SelectionChanged);
    }

    pub fn nudge(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let (nodes, marks) = self.split_selection();
        if nodes.is_empty() && marks.is_empty() {
            return;
        }
        self.editor.nudge(&nodes, &marks, dx, dy);
        self.changed(cx);
    }

    pub fn align(&mut self, how: &str, cx: &mut Context<Self>) {
        let (node_ids, mark_ids) = self.split_selection();
        if node_ids.len() + mark_ids.len() < 2 {
            return;
        }
        #[derive(Clone)]
        struct Item {
            id: String,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            node: bool,
        }
        let mut items = Vec::new();
        for id in &node_ids {
            let Some(node) = self.editor.graph().node(id) else {
                continue;
            };
            let at = node.position_point();
            items.push(Item {
                id: id.clone(),
                x: at.x,
                y: at.y,
                w: card_width(&node.feature),
                h: card_height(node),
                node: true,
            });
        }
        for id in &mark_ids {
            let Some(mark) = self.editor.graph().mark(id) else {
                continue;
            };
            let size = mark.size.unwrap_or([140.0, 80.0]);
            items.push(Item {
                id: id.clone(),
                x: mark.position[0],
                y: mark.position[1],
                w: size[0],
                h: size[1],
                node: false,
            });
        }
        if items.len() < 2 {
            return;
        }
        let min_x = items.iter().map(|item| item.x).fold(f32::MAX, f32::min);
        let min_y = items.iter().map(|item| item.y).fold(f32::MAX, f32::min);
        let max_r = items
            .iter()
            .map(|item| item.x + item.w)
            .fold(f32::MIN, f32::max);
        let max_b = items
            .iter()
            .map(|item| item.y + item.h)
            .fold(f32::MIN, f32::max);
        let mid_x = (min_x + max_r) * 0.5;
        let mid_y = (min_y + max_b) * 0.5;
        if how == "distribute-h" || how == "distribute-v" {
            let horizontal = how == "distribute-h";
            items.sort_by(|a, b| {
                let (left, right) = if horizontal { (a.x, b.x) } else { (a.y, b.y) };
                left.partial_cmp(&right)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if items.len() >= 3 {
                let first = if horizontal { items[0].x } else { items[0].y };
                let last = items.last().unwrap();
                let end = if horizontal { last.x } else { last.y };
                let step = (end - first) / (items.len() - 1) as f32;
                for (index, item) in items.iter_mut().enumerate() {
                    if horizontal {
                        item.x = first + step * index as f32;
                    } else {
                        item.y = first + step * index as f32;
                    }
                }
            }
        } else {
            for item in &mut items {
                match how {
                    "left" => item.x = min_x,
                    "right" => item.x = max_r - item.w,
                    "hcenter" => item.x = mid_x - item.w * 0.5,
                    "top" => item.y = min_y,
                    "bottom" => item.y = max_b - item.h,
                    "vcenter" => item.y = mid_y - item.h * 0.5,
                    _ => {}
                }
            }
        }
        let mut nodes = BTreeMap::new();
        let mut marks = BTreeMap::new();
        for item in items {
            if item.node {
                nodes.insert(item.id, Point::new(item.x, item.y));
            } else {
                marks.insert(item.id, Point::new(item.x, item.y));
            }
        }
        self.editor.place(&nodes, &marks, false);
        self.changed(cx);
    }

    pub fn reorder_selection(&mut self, front: bool, cx: &mut Context<Self>) {
        let (nodes, marks) = self.split_selection();
        self.editor.reorder(&nodes, &marks, front);
        self.changed(cx);
    }

    pub fn start_label_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.editor.graph().mark(id).map(|mark| mark.text.clone()) else {
            return;
        };
        let id = id.to_string();
        self.label_input.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        self.editing = Some(id);
        cx.notify();
    }

    pub fn set_frames(
        &mut self,
        ids: &[String],
        columns: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
        cx: &mut Context<Self>,
    ) {
        let frame = FrameView { columns, rows };
        for id in ids {
            self.frames.insert(id.clone(), frame.clone());
        }
        cx.notify();
    }

    pub fn commit_label(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.editing.clone() else {
            return;
        };
        let text = self.label_input.read(cx).value().to_string();
        self.editor.set_mark_text(&id, text);
        self.editing = None;
        self.changed(cx);
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

    pub fn fit_all(&mut self, cx: &mut Context<Self>) {
        self.fit_ids(false, cx);
    }

    pub fn fit(&mut self, cx: &mut Context<Self>) {
        self.fit_ids(true, cx);
    }

    fn fit_ids(&mut self, selection_only: bool, cx: &mut Context<Self>) {
        let bounds = self
            .viewport
            .borrow()
            .map(|bounds| (bounds.size.width.as_f32(), bounds.size.height.as_f32()));
        let Some((w, h)) = bounds else {
            return;
        };
        let ids: Vec<String> = if self.selection.is_empty() || !selection_only {
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
        self.animate_camera(graph::fit_camera(w, h, rect, 72.0), cx);
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

    /// Move an in-progress link drag. Used by the screenshot reel.
    pub fn show_link(&mut self, from: &str, from_port: &str, at: Point, cx: &mut Context<Self>) {
        self.drag = Some(Drag::Link {
            from: from.to_string(),
            from_port: from_port.to_string(),
            detached: None,
            pointer: at,
        });
        self.refresh_link_status();
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
            pointer: self.canvas_of(point(px(local.0), px(local.1))),
        });
        self.refresh_link_status();
        cx.notify();
    }

    pub fn set_space_hand(&mut self, down: bool, cx: &mut Context<Self>) {
        self.space_hand = down;
        cx.notify();
    }

    fn select_hit(&mut self, local: GPoint<Pixels>) {
        let canvas = self.canvas_of(local);
        for node in self.editor.graph().nodes.iter().rev() {
            let at = node.position_point();
            let height = card_height(node);
            if canvas.x >= at.x
                && canvas.x <= at.x + card_width(&node.feature)
                && canvas.y >= at.y
                && canvas.y <= at.y + height
            {
                self.selection = vec![node.id.clone()];
                return;
            }
        }
        for mark in self.editor.graph().marks.iter().rev() {
            let size = mark.size.unwrap_or([140.0, 80.0]);
            if canvas.x >= mark.position[0]
                && canvas.x <= mark.position[0] + size[0]
                && canvas.y >= mark.position[1]
                && canvas.y <= mark.position[1] + size[1]
            {
                self.selection = vec![mark.id.clone()];
                return;
            }
        }
        self.selection.clear();
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
        if event.button == MouseButton::Right {
            self.select_hit(local);
            cx.emit(CanvasEvent::SelectionChanged);
            cx.notify();
            return;
        }
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
                pointer: self.canvas_of(local),
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
                    pointer: self.canvas_of(local),
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
                self.camera_anim = None;
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
                *pointer = self.canvas_of(local);
                refresh_link = true;
            }
            Drag::Marquee { current, .. } => {
                *current = local;
            }
            Drag::Mark { current, .. } => {
                *current = self.canvas_of(local);
            }
            Drag::Resize {
                id,
                origin_local,
                start_size,
            } => {
                let zoom = self.editor.graph().camera.zoom.max(0.05);
                let dx = (local.x.as_f32() - origin_local.x.as_f32()) / zoom;
                let dy = (local.y.as_f32() - origin_local.y.as_f32()) / zoom;
                let id = id.clone();
                let width = (start_size[0] + dx).max(24.0);
                let mut height = (start_size[1] + dy).max(24.0);
                if event.modifiers.shift {
                    let aspect = start_size[1] / start_size[0].max(1.0);
                    height = (width * aspect).max(24.0);
                }
                self.editor.resize_mark(&id, [width, height]);
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
                if dx.abs() + dy.abs() < 4.0
                    && nodes.is_empty()
                    && marks.len() == 1
                    && let Some((id, _)) = marks.iter().next()
                    && let Some(mark) = self.editor.graph().mark(id).cloned()
                    && mark.kind == MarkKind::Button
                    && let Some(focus) = mark.focus.clone()
                {
                    self.selection = vec![focus];
                    cx.emit(CanvasEvent::SelectionChanged);
                }
                self.editor.commit_gesture();
                if !self.reduced_motion
                    && offsets
                        .values()
                        .any(|offset| offset.x.abs() + offset.y.abs() > 0.5)
                {
                    self.settle = Some(Settle {
                        offsets,
                        started: Instant::now(),
                        duration: motion::SETTLE,
                    });
                }
                self.changed(cx);
            }
            Some(Drag::Link {
                from,
                from_port,
                detached,
                pointer,
            }) => {
                let snap = self.snap_at(pointer, &from, &from_port);
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
                            w: card_width(&node.feature) * zoom,
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
            Some(Drag::Resize { .. }) => {
                self.editor.commit_gesture();
                self.changed(cx);
            }
            Some(Drag::Mark {
                kind,
                start,
                current,
            }) => {
                self.editor.cancel_gesture();
                let x = start.x.min(current.x);
                let y = start.y.min(current.y);
                let mut w = (start.x - current.x).abs().max(16.0);
                let mut h = (start.y - current.y).abs().max(16.0);
                if event.modifiers.shift {
                    let side = w.max(h);
                    w = side;
                    h = side;
                }
                if let Some(id) = self.editor.add_mark(kind, Point::new(x, y), Some([w, h])) {
                    self.selection = vec![id];
                    cx.emit(CanvasEvent::SelectionChanged);
                }
                self.changed(cx);
            }
            None => {}
        }
    }

    fn snap_at(&self, pointer: Point, from: &str, from_port: &str) -> Option<(String, String)> {
        let zoom = self.editor.graph().camera.zoom.max(0.05);
        let mut candidates = Vec::new();
        for node in &self.editor.graph().nodes {
            if node.id == from {
                continue;
            }
            for slot in port_slots(node) {
                if slot.side != Side::In {
                    continue;
                }
                let dx = (slot.center.x - pointer.x) * zoom;
                let dy = (slot.center.y - pointer.y) * zoom;
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
        let snap = self.snap_at(pointer, &from, &from_port);
        if let Some((to, to_port)) = snap {
            let (left, right) = type_pair_label(self.editor.graph(), &from, &to, &to_port);
            self.status = format!("{} → {}", shorten_types(&left), shorten_types(&right));
            return;
        }
        if let Some((node, port, reason)) = self.nearest_reject(pointer, &from, &from_port) {
            let (left, right) = type_pair_label(self.editor.graph(), &from, &node, &port);
            self.status = reject_message(reason, &shorten_types(&left), &shorten_types(&right));
        } else {
            self.status.clear();
        }
    }

    fn nearest_reject(
        &self,
        pointer: Point,
        from: &str,
        from_port: &str,
    ) -> Option<(String, String, Reject)> {
        let zoom = self.editor.graph().camera.zoom.max(0.05);
        let mut best: Option<(f32, String, String, Reject)> = None;
        for node in &self.editor.graph().nodes {
            if node.id == from {
                continue;
            }
            for slot in port_slots(node) {
                if slot.side != Side::In {
                    continue;
                }
                let distance = (((slot.center.x - pointer.x) * zoom).powi(2)
                    + ((slot.center.y - pointer.y) * zoom).powi(2))
                .sqrt();
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

    fn enter_node(&mut self, id: &str) {
        if self.reduced_motion {
            return;
        }
        let mut offsets = BTreeMap::new();
        offsets.insert(id.to_string(), Point::new(0.0, 12.0));
        self.settle = Some(Settle {
            offsets,
            started: Instant::now(),
            duration: motion::ENTER,
        });
    }

    fn animate_camera(&mut self, to: Camera, cx: &mut Context<Self>) {
        let from = self.editor.graph().camera;
        if self.reduced_motion
            || ((from.x - to.x).abs() < 0.5
                && (from.y - to.y).abs() < 0.5
                && (from.zoom - to.zoom).abs() < 0.001)
        {
            self.camera_anim = None;
            self.editor.set_camera(to);
            cx.emit(CanvasEvent::Edited);
            cx.notify();
            return;
        }
        self.camera_anim = Some(CameraAnim {
            from,
            to,
            started: Instant::now(),
        });
        cx.notify();
    }

    fn motion_active(&self) -> bool {
        self.settle.is_some()
            || self.camera_anim.is_some()
            || (self.dots.values().any(|dot| *dot == Dot::Running) && !self.reduced_motion)
    }

    fn tick_motion(&mut self) {
        if let Some(settle) = &self.settle
            && settle.started.elapsed() >= settle.duration
        {
            self.settle = None;
        }
        if let Some(anim) = &self.camera_anim {
            let t = (anim.started.elapsed().as_secs_f32() / motion::CAMERA.as_secs_f32())
                .clamp(0.0, 1.0);
            self.editor.set_camera(Camera {
                x: motion::lerp(anim.from.x, anim.to.x, t),
                y: motion::lerp(anim.from.y, anim.to.y, t),
                zoom: motion::lerp(anim.from.zoom, anim.to.zoom, t),
            });
            if t >= 1.0 {
                self.camera_anim = None;
            }
        }
        if self.dots.values().any(|dot| *dot == Dot::Running) && !self.reduced_motion {
            self.flow = (self.flow + 0.02) % 1.0;
        }
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
                .unwrap_or(*pointer)
        } else {
            *pointer
        };
        Some(LinkVisual {
            from: slot.center,
            to: end,
            snapped: snap,
            reject: reject.map(|(_, _, reason)| reason),
            message: self.status.clone(),
        })
    }
}

#[derive(Clone)]
struct Clip {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    marks: Vec<graph::Mark>,
}

#[derive(Clone)]
struct LinkVisual {
    from: Point,
    to: Point,
    snapped: Option<(String, String)>,
    reject: Option<Reject>,
    message: String,
}

impl EventEmitter<CanvasEvent> for CanvasModel {}

impl Focusable for CanvasModel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CanvasModel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.motion_active() && !self.frame_armed {
            self.frame_armed = true;
            let this = cx.entity().downgrade();
            window.on_next_frame(move |_window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.frame_armed = false;
                    this.tick_motion();
                    cx.notify();
                });
            });
        }
        let viewport = self.viewport.clone();
        let theme_border = cx.theme().border;
        let theme_muted = cx.theme().muted_foreground;
        let theme_primary = cx.theme().primary;
        let theme_danger = cx.theme().danger;
        let theme_bg = cx.theme().background;
        let theme_fg = cx.theme().foreground;
        let theme_card = cx.theme().background;
        let theme_raised = cx.theme().secondary;
        let theme_success = cx.theme().success;
        let theme_warning = cx.theme().warning;
        let chart = [
            cx.theme().chart_1,
            cx.theme().chart_2,
            cx.theme().chart_3,
            cx.theme().chart_4,
            cx.theme().chart_5,
            cx.theme().chart_5,
        ];
        let radius = cx.theme().radius;
        let mono = cx.theme().mono_font_family.clone();

        let camera = self.editor.graph().camera;
        let nodes = self.editor.graph().nodes.clone();
        let edges = self.editor.graph().edges.clone();
        let marks = self.editor.graph().marks.clone();
        let mark_views = marks.clone();
        let selection = self.selection.clone();
        let link = self.link_visual();
        let marquee = match &self.drag {
            Some(Drag::Marquee { start, current, .. }) => Some((*start, *current)),
            _ => None,
        };
        let dragging: Vec<String> = match &self.drag {
            Some(Drag::Move { nodes, .. }) => nodes.keys().cloned().collect(),
            _ => Vec::new(),
        };
        let settle_ids: Vec<(String, Point)> = nodes
            .iter()
            .map(|node| {
                let mut offset = self.settle_offset(&node.id);
                if dragging.iter().any(|id| id == &node.id) && !self.reduced_motion {
                    offset.y -= 3.0 / camera.zoom.max(0.05);
                }
                (node.id.clone(), offset)
            })
            .collect();
        let link_from = match &self.drag {
            Some(Drag::Link {
                from, from_port, ..
            }) => Some((from.clone(), from_port.clone())),
            _ => None,
        };
        let snap_target = link.as_ref().and_then(|item| item.snapped.clone());
        let flow = if self.dots.values().any(|dot| *dot == Dot::Running) && !self.reduced_motion {
            Some(self.flow)
        } else {
            None
        };
        let graph = self.editor.graph().clone();
        let hovered = self.hovered.clone();

        let entity = cx.entity();
        div()
            .id("node-canvas")
            .key_context("Canvas")
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                if this.editing.is_some() {
                    return;
                }
                let key: &str = event.keystroke.key.as_ref();
                let platform = event.keystroke.modifiers.platform;
                let step = if event.keystroke.modifiers.alt {
                    1.0
                } else {
                    16.0
                };
                if platform && key == "c" {
                    this.copy_selection();
                } else if platform && key == "x" {
                    this.cut_selection(cx);
                } else if platform && key == "v" {
                    this.paste(cx);
                } else if platform && key == "d" {
                    this.duplicate_selection(cx);
                } else if platform && key == "]" {
                    this.reorder_selection(true, cx);
                } else if platform && key == "[" {
                    this.reorder_selection(false, cx);
                } else if key == "left" {
                    this.nudge(-step, 0.0, cx);
                } else if key == "right" {
                    this.nudge(step, 0.0, cx);
                } else if key == "up" {
                    this.nudge(0.0, -step, cx);
                } else if key == "down" {
                    this.nudge(0.0, step, cx);
                } else if platform && key == "l" {
                    this.align("left", cx);
                } else if platform && key == "e" {
                    this.align("hcenter", cx);
                } else if platform && event.keystroke.modifiers.shift && key == "h" {
                    this.align("distribute-h", cx);
                }
            }))
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
                    this.camera_anim = None;
                    let camera = graph::zoom_toward(
                        this.editor.graph().camera,
                        Point::new(local.x.as_f32(), local.y.as_f32()),
                        factor,
                    );
                    this.editor.set_camera(camera);
                    cx.emit(CanvasEvent::Edited);
                    cx.notify();
                } else {
                    this.camera_anim = None;
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
            .child({
                let painted_selection = selection.clone();
                let painted_chart = chart;
                let painted_settle = settle_ids.clone();
                let painted_link = link.clone();
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
                            &painted_settle,
                            painted_link.as_ref(),
                            flow,
                            theme_muted,
                            theme_primary,
                            theme_danger,
                            &painted_chart,
                            window,
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full()
            })
            .children(self.editor.graph().nodes.iter().map(|node| {
                let offset = settle_ids
                    .iter()
                    .find(|(id, _)| id == &node.id)
                    .map(|(_, offset)| *offset)
                    .unwrap_or_else(|| Point::new(0.0, 0.0));
                card_element(
                    entity.clone(),
                    node,
                    &graph,
                    self.dots.get(&node.id).copied().unwrap_or(if node.unknown {
                        Dot::Danger
                    } else {
                        Dot::Muted
                    }),
                    self.reduced_motion,
                    camera,
                    offset,
                    selection.iter().any(|id| id == &node.id),
                    hovered.as_deref() == Some(node.id.as_str()),
                    dragging.iter().any(|id| id == &node.id),
                    link_from.clone(),
                    snap_target.clone(),
                    CardPaint {
                        card: theme_card,
                        raised: theme_raised,
                        fg: theme_fg,
                        muted: theme_muted,
                        primary: theme_primary,
                        border: theme_border,
                        success: theme_success,
                        warning: theme_warning,
                        danger: theme_danger,
                        chart,
                        radius,
                        mono: mono.to_string(),
                        frame: self.frames.get(&node.id).cloned(),
                    },
                )
            }))
            .children(mark_views.iter().map(|mark| {
                mark_element(
                    entity.clone(),
                    mark,
                    camera,
                    selection.iter().any(|id| id == &mark.id),
                    theme_fg,
                    theme_muted,
                    theme_border,
                    theme_card,
                    theme_primary,
                )
            }))
            .when_some(self.editing.clone(), |this, id| {
                let Some(mark) = self.editor.graph().mark(&id) else {
                    return this;
                };
                let at = camera.to_window(Point::new(mark.position[0], mark.position[1]));
                let width = mark.size.map(|size| size[0]).unwrap_or(168.0) * camera.zoom;
                this.child(
                    div()
                        .absolute()
                        .left(px(at.x))
                        .top(px(at.y))
                        .w(px(width))
                        .bg(theme_card)
                        .border_1()
                        .border_color(theme_primary)
                        .rounded(px(6.0))
                        .p_1()
                        .child(Input::new(&self.label_input)),
                )
            })
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
            .when(
                link.as_ref()
                    .is_some_and(|item| item.reject.is_some() && !item.message.is_empty()),
                |this| {
                    let item = link.clone().expect("link");
                    let at = camera.to_window(item.to);
                    let text = item.message;
                    this.child(
                        div()
                            .absolute()
                            .left(px(at.x + 10.0))
                            .top(px((at.y - 28.0).max(8.0)))
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .bg(theme_fg)
                            .text_color(theme_card)
                            .shadow(card_shadow(false, theme_fg))
                            .text_xs()
                            .child(text),
                    )
                },
            )
            .context_menu({
                let entity = cx.entity();
                move |menu, _, _| canvas_menu(menu, entity.clone())
            })
    }
}

fn canvas_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: Entity<CanvasModel>,
) -> gpui_kit::component::menu::PopupMenu {
    let paste = entity.clone();
    let duplicate = entity.clone();
    let delete = entity.clone();
    let align = entity.clone();
    let front = entity.clone();
    let back = entity.clone();
    let edit = entity.clone();
    menu.item(PopupMenuItem::new("Paste").on_click(move |_, _, cx| {
        paste.update(cx, |this, cx| this.paste(cx));
    }))
    .item(PopupMenuItem::new("Duplicate").on_click(move |_, _, cx| {
        duplicate.update(cx, |this, cx| this.duplicate_selection(cx));
    }))
    .item(PopupMenuItem::new("Delete").on_click(move |_, _, cx| {
        delete.update(cx, |this, cx| this.delete_selection(cx));
    }))
    .separator()
    .item(PopupMenuItem::new("Align left").on_click(move |_, _, cx| {
        align.update(cx, |this, cx| this.align("left", cx));
    }))
    .item(
        PopupMenuItem::new("Bring to front").on_click(move |_, _, cx| {
            front.update(cx, |this, cx| this.reorder_selection(true, cx));
        }),
    )
    .item(
        PopupMenuItem::new("Send to back").on_click(move |_, _, cx| {
            back.update(cx, |this, cx| this.reorder_selection(false, cx));
        }),
    )
    .item(
        PopupMenuItem::new("Edit label").on_click(move |_, window, cx| {
            edit.update(cx, |this, cx| {
                if let Some(id) = this.selection.first().cloned() {
                    this.start_label_edit(&id, window, cx);
                }
            });
        }),
    )
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

struct CardPaint {
    card: gpui_kit::Hsla,
    raised: gpui_kit::Hsla,
    fg: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    success: gpui_kit::Hsla,
    warning: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    chart: [gpui_kit::Hsla; 6],
    radius: Pixels,
    mono: String,
    frame: Option<FrameView>,
}

fn type_color(ty: TypeName, chart: &[gpui_kit::Hsla; 6]) -> gpui_kit::Hsla {
    match ty {
        TypeName::Dataframe => chart[0],
        TypeName::Series => chart[1],
        TypeName::Scalar => chart[2],
        TypeName::List | TypeName::Index => chart[3],
        TypeName::Dict | TypeName::Ndarray => chart[4],
        TypeName::Code => chart[5],
    }
}

fn group_color(group: &str, chart: &[gpui_kit::Hsla; 6]) -> gpui_kit::Hsla {
    match group {
        "IO" => chart[0],
        "Labels" => chart[1],
        "Select" => chart[2],
        "Shape" => chart[3],
        "Code" => chart[4],
        _ => chart[5],
    }
}

fn shorten_types(label: &str) -> String {
    label
        .replace("dataframe", "table")
        .replace("series", "column")
        .replace("ndarray", "array")
        .replace("scalar", "value")
        .replace("dict", "map")
}

fn port_caption(name: &str, accepts: &[TypeName]) -> String {
    let labels: Vec<&str> = accepts.iter().copied().map(TypeName::short_label).collect();
    let kind = if labels.is_empty() {
        "value".to_string()
    } else if labels.contains(&"table") {
        "table".to_string()
    } else {
        labels[0].to_string()
    };
    if name == "in" || name == "out" || name.is_empty() {
        kind
    } else {
        format!("{name} · {kind}")
    }
}

fn shifted(nodes: &[Node], offsets: &[(String, Point)]) -> Vec<Node> {
    nodes
        .iter()
        .cloned()
        .map(|mut node| {
            if let Some((_, offset)) = offsets.iter().find(|(id, _)| id == &node.id) {
                node.position[0] += offset.x;
                node.position[1] += offset.y;
            }
            node
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn paint_scene(
    bounds: Bounds<Pixels>,
    camera: Camera,
    nodes: &[Node],
    edges: &[Edge],
    _marks: &[graph::Mark],
    selection: &[String],
    settle: &[(String, Point)],
    link: Option<&LinkVisual>,
    flow: Option<f32>,
    muted: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    chart: &[gpui_kit::Hsla; 6],
    window: &mut Window,
    _cx: &mut App,
) {
    let origin = bounds.origin;
    let zoom = camera.zoom;
    if zoom > 0.35 {
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
            window.paint_path(path, muted.opacity(0.45));
        }
    }

    let placed = shifted(nodes, settle);
    for edge in edges {
        let Some(cubic) = layout::edge_cubic(&placed, edge) else {
            continue;
        };
        let selected = selection.iter().any(|id| id == &edge.id);
        let ty = output_types_first(nodes, &edge.from);
        let color = ty
            .map(|ty| type_color(ty, chart))
            .unwrap_or(muted)
            .opacity(if selected { 1.0 } else { 0.85 });
        paint_cubic(
            origin,
            camera,
            cubic,
            color,
            px(if selected { 1.75 } else { 1.25 }),
            window,
        );
        if let Some(phase) = flow {
            let at = layout::cubic_point(cubic, phase);
            paint_dot(origin, camera, at, 3.0, color, window);
        }
    }
    if let Some(link) = link {
        let cubic = layout::wire_cubic(link.from, link.to);
        let color = if link.reject.is_some() {
            danger
        } else if link.snapped.is_some() {
            link_source_color(nodes, link, chart, primary)
        } else {
            muted
        };
        paint_cubic(origin, camera, cubic, color, px(1.5), window);
    }
}

fn output_types_first(nodes: &[Node], id: &str) -> Option<TypeName> {
    let graph = graph::Graph {
        nodes: nodes.to_vec(),
        ..graph::Graph::default()
    };
    output_types(&graph, id).into_iter().next()
}

fn link_source_color(
    nodes: &[Node],
    link: &LinkVisual,
    chart: &[gpui_kit::Hsla; 6],
    fallback: gpui_kit::Hsla,
) -> gpui_kit::Hsla {
    // The source id is not on LinkVisual. Color from the node whose output slot matches `from`.
    for node in nodes {
        for slot in port_slots(node) {
            if slot.side == Side::Out
                && (slot.center.x - link.from.x).abs() < 0.5
                && (slot.center.y - link.from.y).abs() < 0.5
            {
                return output_types_first(nodes, &node.id)
                    .map(|ty| type_color(ty, chart))
                    .unwrap_or(fallback);
            }
        }
    }
    fallback
}

fn paint_dot(
    origin: GPoint<Pixels>,
    camera: Camera,
    at: Point,
    radius: f32,
    color: gpui_kit::Hsla,
    window: &mut Window,
) {
    let mapped = camera.to_window(at);
    let center = origin + point(px(mapped.x), px(mapped.y));
    let mut path = gpui_kit::PathBuilder::fill();
    path.move_to(center + point(px(radius), px(0.0)));
    // A small diamond reads as a bead without a full arc API.
    path.line_to(center + point(px(0.0), px(radius)));
    path.line_to(center + point(px(-radius), px(0.0)));
    path.line_to(center + point(px(0.0), px(-radius)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
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

fn card_shadow(dragging: bool, ink: gpui_kit::Hsla) -> Vec<gpui_kit::BoxShadow> {
    if dragging {
        vec![
            gpui_kit::BoxShadow::new(px(0.), px(8.), ink.opacity(0.10)).blur_radius(px(16.)),
            gpui_kit::BoxShadow::new(px(0.), px(1.), ink.opacity(0.08)).blur_radius(px(2.)),
        ]
    } else {
        vec![
            gpui_kit::BoxShadow::new(px(0.), px(1.), ink.opacity(0.06)).blur_radius(px(2.)),
            gpui_kit::BoxShadow::new(px(0.), px(1.), ink.opacity(0.04)).blur_radius(px(1.)),
        ]
    }
}

#[allow(clippy::too_many_arguments)]
fn card_element(
    entity: Entity<CanvasModel>,
    node: &Node,
    graph: &graph::Graph,
    dot_kind: Dot,
    reduced_motion: bool,
    camera: Camera,
    offset: Point,
    selected: bool,
    hovered: bool,
    dragging: bool,
    link_from: Option<(String, String)>,
    snap_target: Option<(String, String)>,
    paint: CardPaint,
) -> gpui_kit::AnyElement {
    let zoom = camera.zoom;
    let origin = node.position_point();
    let window = camera.to_window(Point::new(origin.x + offset.x, origin.y + offset.y));
    let width = px(card_width(&node.feature) * zoom);
    let id = node.id.clone();
    let title = node.label.clone();
    let summary = node.summary();
    let inputs = node.input_ports();
    let output = node.output_port();
    let rows = inputs.len().max(usize::from(output.is_some())).max(1);
    let accent = node
        .def()
        .map(|def| group_color(def.group, &paint.chart))
        .unwrap_or(paint.muted);
    let pulse = if dot_kind == Dot::Running && !reduced_motion {
        let wave = (Instant::now().elapsed().as_secs_f32() * 4.0).sin().abs();
        0.45 + 0.55 * wave
    } else {
        1.0
    };
    let dot = match dot_kind {
        Dot::Success => paint.success,
        Dot::Warning => paint.warning,
        Dot::Danger => paint.danger,
        Dot::Running => paint.primary,
        Dot::Muted => paint.muted.opacity(0.55),
    }
    .opacity(pulse);
    let border_color = if dot_kind == Dot::Danger {
        paint.danger
    } else if selected {
        paint.primary
    } else if hovered {
        paint.fg.opacity(0.28)
    } else {
        paint.border
    };
    let bg = if selected || hovered || dragging {
        paint.raised
    } else {
        paint.card
    };

    div()
        .id(SharedString::from(format!("card-{id}")))
        .absolute()
        .left(px(window.x))
        .top(px(window.y))
        .w(width)
        .flex()
        .flex_col()
        .rounded(paint.radius)
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .shadow(card_shadow(dragging, paint.fg))
        .overflow_hidden()
        .on_hover({
            let entity = entity.clone();
            let id = id.clone();
            move |over, _, cx| {
                entity.update(cx, |this, cx| {
                    if *over {
                        this.hovered = Some(id.clone());
                    } else if this.hovered.as_deref() == Some(id.as_str()) {
                        this.hovered = None;
                    }
                    cx.notify();
                });
            }
        })
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
                .h(px(layout::HEADER * zoom))
                .mt(px(layout::PAD_Y * zoom))
                .px(px(10.0 * zoom))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0 * zoom))
                .child(
                    div()
                        .w(px(3.0 * zoom))
                        .h(px(14.0 * zoom))
                        .rounded(px(2.0))
                        .bg(accent),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_sm()
                        .text_color(paint.fg)
                        .child(title),
                )
                .child(div().size(px(7.0 * zoom)).rounded_full().bg(dot)),
        )
        .child(
            div()
                .h(px(layout::SUMMARY * zoom))
                .px(px(10.0 * zoom))
                .overflow_hidden()
                .text_xs()
                .text_color(paint.muted)
                .font_family(paint.mono.clone())
                .child(summary),
        )
        .child({
            let block = layout::PORT_ROW * rows as f32;
            let mut column = div()
                .relative()
                .mb(px(layout::PAD_Y * zoom))
                .h(px(block * zoom));
            if inputs.len() <= 1 {
                column = column.child(port_pair(
                    entity.clone(),
                    &id,
                    inputs.first(),
                    output.as_ref(),
                    0.0,
                    zoom,
                    graph,
                    link_from.clone(),
                    snap_target.clone(),
                    &paint,
                ));
            } else {
                for (index, port) in inputs.iter().enumerate() {
                    column = column.child(port_pair(
                        entity.clone(),
                        &id,
                        Some(port),
                        None,
                        layout::PORT_ROW * index as f32,
                        zoom,
                        graph,
                        link_from.clone(),
                        snap_target.clone(),
                        &paint,
                    ));
                }
                if let Some(port) = output.as_ref() {
                    let top = (block - layout::PORT_ROW) * 0.5;
                    column = column.child(port_pair(
                        entity.clone(),
                        &id,
                        None,
                        Some(port),
                        top,
                        zoom,
                        graph,
                        None,
                        None,
                        &paint,
                    ));
                }
            }
            column
        })
        .children(output_body(
            node,
            paint.frame.as_ref(),
            selected,
            reduced_motion,
            &paint,
        ))
        .into_any_element()
}

fn output_body(
    node: &Node,
    frame: Option<&FrameView>,
    selected: bool,
    reduced_motion: bool,
    paint: &CardPaint,
) -> Option<gpui_kit::AnyElement> {
    if node.feature == "preview_table" {
        return Some(match frame {
            Some(frame) => mini_table(frame, paint),
            None => div()
                .px(px(10.0))
                .text_xs()
                .text_color(paint.muted)
                .child("Run preview")
                .into_any_element(),
        });
    }
    if node.feature != "chart" {
        return None;
    }
    let Some(frame) = frame else {
        return Some(
            div()
                .px(px(10.0))
                .text_xs()
                .text_color(paint.muted)
                .child("Run preview")
                .into_any_element(),
        );
    };
    let mark = node.param_str("mark");
    let colors = [
        paint.chart[0],
        paint.chart[1],
        paint.chart[2],
        paint.chart[3],
        paint.chart[4],
    ];
    Some(
        match points_from(
            &frame.columns,
            &frame.rows,
            &node.param_str("x"),
            &node.param_str("y"),
            &node.param_str("series"),
        ) {
            Ok(points) => div()
                .h(px(168.0))
                .px(px(6.0))
                .pb(px(6.0))
                .child(feature_chart(
                    &mark,
                    points,
                    colors,
                    paint.fg,
                    reduced_motion,
                    selected,
                    &node.id,
                ))
                .into_any_element(),
            Err(message) => div()
                .px(px(10.0))
                .text_xs()
                .text_color(paint.danger)
                .child(message)
                .into_any_element(),
        },
    )
}

fn mini_table(frame: &FrameView, paint: &CardPaint) -> gpui_kit::AnyElement {
    let count = frame.columns.len().min(4);
    let mut grid = div().px(px(8.0)).pb(px(6.0)).flex().flex_col().gap(px(2.0));
    grid = grid.child(mini_row(
        &frame
            .columns
            .iter()
            .take(count)
            .cloned()
            .collect::<Vec<_>>(),
        true,
        paint,
    ));
    for row in frame.rows.iter().take(4) {
        let cells = (0..count)
            .map(|index| row.get(index).map(graph::cell_text).unwrap_or_default())
            .collect::<Vec<_>>();
        grid = grid.child(mini_row(&cells, false, paint));
    }
    grid.into_any_element()
}

fn mini_row(cells: &[String], header: bool, paint: &CardPaint) -> gpui_kit::Div {
    let mut row = div().flex().flex_row().gap(px(6.0));
    for cell in cells {
        let numeric = !header && cell.parse::<f64>().is_ok();
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_xs()
                .when(numeric, |cell| cell.text_right())
                .font_family(paint.mono.clone())
                .text_color(if header { paint.fg } else { paint.muted })
                .child(cell.clone()),
        );
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn port_pair(
    entity: Entity<CanvasModel>,
    node_id: &str,
    input: Option<&graph::PortRecord>,
    output: Option<&graph::PortRecord>,
    top: f32,
    zoom: f32,
    graph: &graph::Graph,
    link_from: Option<(String, String)>,
    snap_target: Option<(String, String)>,
    paint: &CardPaint,
) -> gpui_kit::AnyElement {
    let node_key = node_id.to_string();
    let mut row = div()
        .id(SharedString::from(format!(
            "ports-{}-{}",
            node_key, top as i32
        )))
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .top(px(top * zoom))
        .h(px(layout::PORT_ROW * zoom))
        .px(px(8.0 * zoom))
        .flex()
        .flex_row()
        .items_center()
        .justify_between();
    if let Some(port) = input {
        row = row.child(port_hit(
            entity.clone(),
            node_id,
            port,
            Side::In,
            zoom,
            graph,
            link_from.clone(),
            snap_target.clone(),
            paint,
        ));
    } else {
        row = row.child(div());
    }
    if let Some(port) = output {
        row = row.child(port_hit(
            entity,
            node_id,
            port,
            Side::Out,
            zoom,
            graph,
            None,
            None,
            paint,
        ));
    }
    row.into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn port_hit(
    entity: Entity<CanvasModel>,
    node_id: &str,
    port: &graph::PortRecord,
    side: Side,
    zoom: f32,
    graph: &graph::Graph,
    link_from: Option<(String, String)>,
    snap_target: Option<(String, String)>,
    paint: &CardPaint,
) -> gpui_kit::AnyElement {
    let name = port.name.clone();
    let caption = port_caption(&port.name, &port.accepts);
    let ty = port.accepts.first().copied().unwrap_or(TypeName::Dataframe);
    let color = type_color(ty, &paint.chart);
    let compatible = match (&link_from, side) {
        (Some((from, from_port)), Side::In) => {
            try_connect(graph, from, from_port, node_id, &port.name).is_ok()
        }
        _ => true,
    };
    let snapped = snap_target
        .as_ref()
        .is_some_and(|(id, port_name)| id == node_id && port_name == &port.name);
    let dim = link_from.is_some() && side == Side::In && !compatible;
    let dot = div()
        .size(px(if snapped { 11.0 } else { 8.0 } * zoom))
        .flex_shrink_0()
        .rounded_full()
        .bg(if dim {
            paint.muted.opacity(0.35)
        } else {
            color
        })
        .border_1()
        .border_color(if snapped {
            paint.primary
        } else {
            color.opacity(0.45)
        });
    let label = div()
        .max_w(px(78.0 * zoom))
        .overflow_hidden()
        .text_xs()
        .text_color(if dim {
            paint.muted.opacity(0.4)
        } else {
            paint.muted
        })
        .child(caption);
    let node_id = node_id.to_string();
    let row = div()
        .id(SharedString::from(format!(
            "{}-{node_id}-{name}",
            if side == Side::In { "in" } else { "out" }
        )))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0 * zoom))
        .h(px(layout::PORT_ROW * zoom));
    let row = if side == Side::In {
        row.child(dot).child(label)
    } else {
        row.child(label).child(dot)
    };
    row.on_mouse_down(MouseButton::Left, move |event, window, cx| {
        cx.stop_propagation();
        entity.update(cx, |this, cx| {
            this.port_down(&node_id, &name, side, event, window, cx);
        });
    })
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn mark_element(
    entity: Entity<CanvasModel>,
    mark: &graph::Mark,
    camera: Camera,
    selected: bool,
    fg: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    card: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    let zoom = camera.zoom;
    let at = camera.to_window(Point::new(mark.position[0], mark.position[1]));
    let size = mark.size.unwrap_or(match mark.kind {
        MarkKind::Label | MarkKind::Note | MarkKind::Button => [168.0, 36.0],
        MarkKind::Rectangle | MarkKind::Circle => [140.0, 88.0],
    });
    let id = mark.id.clone();
    let placeholder = mark.text.is_empty();
    let text = if placeholder {
        match mark.kind {
            MarkKind::Label => "Label".to_string(),
            MarkKind::Note => "Note".to_string(),
            MarkKind::Button => "Button".to_string(),
            _ => String::new(),
        }
    } else {
        mark.text.clone()
    };
    let shape = matches!(mark.kind, MarkKind::Rectangle | MarkKind::Circle);
    let mut body = div()
        .id(SharedString::from(format!("mark-{id}")))
        .absolute()
        .left(px(at.x))
        .top(px(at.y))
        .w(px(size[0] * zoom))
        .h(px(size[1] * zoom))
        .px(px(8.0 * zoom))
        .flex()
        .items_center()
        .text_sm()
        .text_color(if placeholder { muted } else { fg })
        .when(mark.kind == MarkKind::Circle, |el| el.rounded_full())
        .when(mark.kind == MarkKind::Rectangle, |el| el.rounded(px(8.0)))
        .when(shape, |el| {
            el.border_1()
                .border_color(if selected { primary } else { border })
                .bg(card.opacity(0.35))
        })
        .when(mark.kind == MarkKind::Note, |el| {
            el.rounded(px(8.0))
                .border_1()
                .border_color(if selected { primary } else { border })
                .bg(card)
                .items_start()
                .py(px(6.0))
        })
        .when(mark.kind == MarkKind::Button, |el| {
            el.rounded(px(8.0))
                .border_1()
                .border_color(if selected { primary } else { border })
                .bg(primary.opacity(0.12))
                .justify_center()
        })
        .when(!text.is_empty(), |el| el.child(text))
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            let id = id.clone();
            move |event, window, cx| {
                cx.stop_propagation();
                entity.update(cx, |this, cx| {
                    let kind = this.editor.graph().mark(&id).map(|mark| mark.kind);
                    if event.click_count >= 2
                        && matches!(kind, Some(MarkKind::Label | MarkKind::Note))
                    {
                        this.start_label_edit(&id, window, cx);
                        return;
                    }
                    this.editor.raise_mark(&id);
                    this.card_down(&id, event, window, cx);
                });
            }
        });
    if selected && mark.kind != MarkKind::Label {
        let start = size;
        body = body.child(
            div()
                .absolute()
                .right(px(-2.0))
                .bottom(px(-2.0))
                .size(px(14.0 * zoom.max(0.8)))
                .rounded(px(2.0))
                .bg(primary)
                .on_mouse_down(MouseButton::Left, {
                    let entity = entity.clone();
                    let id = id.clone();
                    move |event, _window, cx| {
                        cx.stop_propagation();
                        entity.update(cx, |this, cx| {
                            this.editor.begin_gesture();
                            let local = this.local_of(event.position);
                            this.drag = Some(Drag::Resize {
                                id: id.clone(),
                                origin_local: local,
                                start_size: start,
                            });
                            cx.notify();
                        });
                    }
                }),
        );
    }
    body.into_any_element()
}
