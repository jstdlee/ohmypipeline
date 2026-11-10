//! Pointer gestures for the canvas. Rules stay in `graph`; this file only
//! decides which rule a gesture is asking for.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
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
    Camera, Document, Edge, Frame, MarkKind, Node, Point, Reject, SNAP_RADIUS_PX, SnapCandidate,
    TypeName, card_width, output_types, pick_snap, reject_message, try_connect, type_pair_label,
};

use crate::layout::{self, Detail, Side, card_height, port_slots};
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
    /// Drag out an empty group frame.
    Frame,
}

impl Tool {
    pub fn is_mark(self) -> bool {
        matches!(
            self,
            Self::Rectangle | Self::Circle | Self::Label | Self::Note | Self::Button
        )
    }
}

/// Run or reset one node. Emitted from the card context menu; the workspace
/// owns the execution state, so it does the work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeCommand {
    /// Run this node as a preview job.
    Execute,
    /// The same run, with the menu name that says where it stops.
    ExecuteTo,
    /// Drop this node's cached output and everything downstream of it.
    Reset,
}

pub enum CanvasEvent {
    Edited,
    SelectionChanged,
    FocusInspector,
    /// Open the node search popover. `from` is set when a drag left an output
    /// port, `to` when it ended on an input side; `at` is the canvas point of
    /// the drop and `screen` its window position.
    OpenCatalog {
        from: Option<(String, String)>,
        to: Option<(String, String)>,
        ty: Option<TypeName>,
        at: Point,
        screen: GPoint<Pixels>,
    },
    /// An Execute, Execute to here, or Reset picked from a card or the menu.
    NodeCommand {
        id: String,
        command: NodeCommand,
    },
    /// The frame on the clicked wire. The wire's source node computes it.
    PreviewWire {
        edge: String,
    },
    /// An output port was clicked without a drag.
    PreviewPort {
        node: String,
    },
}

struct Settle {
    /// Canvas-pixel offset from the committed position, at the start of the settle.
    offsets: BTreeMap<String, Point>,
    started: Instant,
    duration: Duration,
}

/// Identity of a wire's shape in canvas space. Any change re-derives the
/// chain; panning and zooming leave it untouched, only the projection moves.
#[derive(Clone, PartialEq)]
struct WireKey {
    from: Point,
    to: Point,
    reroutes: Vec<Point>,
}

/// A chain waiting for the next paint. Corrected whenever its input key no
/// longer matches, and dropped wholesale on a document edit.
struct CachedWire {
    key: WireKey,
    path: Arc<Vec<layout::Cubic>>,
}

/// Data the paint scene needs per wire, resolved once per render so the
/// hot loop does no graph lookups or vector building.
struct WirePaint {
    id: String,
    reroutes: Vec<Point>,
    path: Arc<Vec<layout::Cubic>>,
    source_color: gpui_kit::Hsla,
}

struct CameraAnim {
    from: Camera,
    to: Camera,
    started: Instant,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum Drag {
    Move {
        origin_local: GPoint<Pixels>,
        nodes: BTreeMap<String, Point>,
        marks: BTreeMap<String, Point>,
        #[allow(dead_code)]
        duplicated: bool,
    },
    Link {
        /// The output-side anchor that stays attached, when the drag keeps one.
        from: Option<(String, String)>,
        /// The input-side anchor the drag started from, when any.
        to: Option<(String, String)>,
        /// Wires picked up by a modifier drag on an output port. They are
        /// detached for the drag; a cancel restores them.
        rewired: Vec<Edge>,
        /// Canvas point under the pointer.
        pointer: Point,
        /// Canvas point where the drag began, to tell a click from a move.
        origin: Point,
    },
    /// Drag of one reroute point on a wire.
    Reroute {
        edge: String,
        index: usize,
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
    /// Dragging out an empty frame with the Frame tool.
    FrameDraw {
        start: Point,
        current: Point,
    },
    /// Header drag. The frame and its members move together, live.
    FrameMove {
        id: String,
        last: Point,
    },
    /// A resize handle on the bottom-right edges of a selected frame.
    FrameResize {
        id: String,
        origin_local: GPoint<Pixels>,
        start_size: [f32; 2],
        edge: crate::frames::ResizeEdge,
    },
}

/// What the inline text input edits: a mark's text or a frame's title.
#[derive(Clone, Debug)]
enum EditTarget {
    Mark(String),
    Frame(String),
}

/// Collapse or expand animation of one frame's visible height.
struct FrameAnim {
    /// Height when the animation started, in canvas pixels.
    from: f32,
    started: Instant,
}

/// How long a badge color blends into the next one.
const BADGE_FADE: Duration = Duration::from_millis(120);

/// What the header badge of a card says. The reason and the message come
/// from the session, which owns the execution state.
#[derive(Clone, Debug, PartialEq)]
pub enum Badge {
    /// Hollow ring. The reason, when it is known, goes in the tooltip.
    NotConfigured(Option<String>),
    /// Filled muted dot.
    Configured,
    /// Accent dot, pulsing while reduced motion is off.
    Running,
    /// Filled success dot.
    Executed,
    /// Filled danger dot, plus a danger border on the card.
    Error(String),
    /// Filled warning dot.
    Stale,
    /// The branch is skipped. The whole card draws at 40 percent opacity.
    Inactive,
}

impl Badge {
    /// Stable index of the badge's color class. A change between two dot
    /// classes fades over 120 ms; a change into or out of the ring, or of
    /// the inactive dimming, jumps straight to the new look.
    fn class(&self) -> u8 {
        match self {
            Self::NotConfigured(_) => 0,
            Self::Configured => 1,
            Self::Running => 2,
            Self::Executed => 3,
            Self::Error(_) => 4,
            Self::Stale => 5,
            Self::Inactive => 6,
        }
    }

    fn is_dot(&self) -> bool {
        matches!(self.class(), 1..=5)
    }

    /// The tooltip text of the badge.
    pub fn tooltip_text(&self) -> String {
        match self {
            Self::NotConfigured(reason) => match reason {
                Some(reason) => format!("Not configured: {reason}"),
                None => "Not configured".to_string(),
            },
            Self::Configured => "Configured".to_string(),
            Self::Running => "Running".to_string(),
            Self::Executed => "Executed".to_string(),
            Self::Error(message) => format!("Error: {message}"),
            Self::Stale => "Stale: an input or setting changed".to_string(),
            Self::Inactive => "Inactive branch".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FrameView {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

pub struct CanvasModel {
    focus: FocusHandle,
    editor: Document,
    badges: BTreeMap<String, Badge>,
    /// Color of the badge before the last change, and when it changed. A
    /// badge color blends into the next one over 120 ms when motion is on.
    badge_fade: BTreeMap<String, (Badge, Instant)>,
    pub(crate) selection: Vec<String>,
    tool: Tool,
    space_hand: bool,
    drag: Option<Drag>,
    settle: Option<Settle>,
    camera_anim: Option<CameraAnim>,
    /// Visible-height animation of collapsing or expanding frames.
    frame_anim: BTreeMap<String, FrameAnim>,
    /// Drag in progress on the minimap; the camera follows immediately.
    minimap_drag: bool,
    /// When the minimap fade started, with the direction it heads.
    minimap_fade: Option<(bool, Instant)>,
    frame_armed: bool,
    hovered: Option<String>,
    /// Wire the pointer is over, with the nearest segment.
    hover_wire: Option<(String, usize)>,
    /// Reroute point the pointer is over.
    hover_reroute: Option<(String, usize)>,
    /// Reroute point the user selected, for Delete.
    picked_reroute: Option<(String, usize)>,
    /// Edge the context menu was opened on, with the pointer point.
    wire_pick: Option<(String, Point, usize)>,
    /// Wire a dragged node could be inserted into.
    insert_hint: Option<String>,
    flow: f32,
    clip: Option<Clip>,
    label_input: Entity<gpui_kit::component::input::InputState>,
    editing: Option<EditTarget>,
    frames: BTreeMap<String, FrameView>,
    notes: BTreeMap<String, String>,
    reduced_motion: bool,
    status: String,
    viewport: Rc<RefCell<Option<Bounds<Pixels>>>>,
    wire_cache: HashMap<String, CachedWire>,
}

impl CanvasModel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let label_input = cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx));
        cx.subscribe(
            &label_input,
            |this, input, event: &gpui_kit::component::input::InputEvent, cx| {
                match event {
                    gpui_kit::component::input::InputEvent::Blur => {
                        if this.editing.is_some() {
                            this.commit_edit_target(&input, cx);
                        }
                    }
                    // Enter commits the inline edit; the input keeps newline
                    // handling for multi-line marks.
                    gpui_kit::component::input::InputEvent::PressEnter { .. }
                        if this.editing.is_some() =>
                    {
                        this.commit_edit_target(&input, cx);
                    }
                    _ => {}
                }
            },
        )
        .detach();
        Self {
            focus: cx.focus_handle(),
            editor: Document::new(),
            badges: BTreeMap::new(),
            badge_fade: BTreeMap::new(),
            selection: Vec::new(),
            tool: Tool::Select,
            space_hand: false,
            drag: None,
            settle: None,
            camera_anim: None,
            frame_anim: BTreeMap::new(),
            minimap_drag: false,
            minimap_fade: None,
            frame_armed: false,
            hovered: None,
            hover_wire: None,
            hover_reroute: None,
            picked_reroute: None,
            wire_pick: None,
            insert_hint: None,
            flow: 0.0,
            clip: None,
            label_input,
            editing: None,
            frames: BTreeMap::new(),
            notes: BTreeMap::new(),
            reduced_motion: false,
            status: String::new(),
            viewport: Rc::new(RefCell::new(None)),
            wire_cache: HashMap::new(),
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

    pub fn set_badges(&mut self, badges: BTreeMap<String, Badge>, cx: &mut Context<Self>) {
        let now = Instant::now();
        let mut gain = Vec::new();
        for (id, badge) in &badges {
            let old = self.badges.get(id).cloned().unwrap_or(Badge::Configured);
            if old == *badge || !old.is_dot() || !badge.is_dot() {
                continue;
            }
            gain.push(id.clone());
            self.badge_fade.insert(id.clone(), (old, now));
        }
        self.badges = badges;
        if !gain.is_empty() {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(BADGE_FADE + Duration::from_millis(40))
                    .await;
                // The motion tick loop paints the blend on frames that run.
                // This timer is the floor: it repaints the settled color even
                // when the window draws no motion frames at all.
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        cx.notify();
    }

    pub fn set_reduced_motion(&mut self, reduced: bool, cx: &mut Context<Self>) {
        self.reduced_motion = reduced;
        if reduced {
            self.settle = None;
            self.badge_fade.clear();
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
        self.frame_anim.clear();
        self.wire_cache.clear();
        self.badge_fade.clear();
        self.badges.clear();
        self.minimap_drag = false;
        self.minimap_fade = None;
        self.hover_wire = None;
        self.hover_reroute = None;
        self.picked_reroute = None;
        self.wire_pick = None;
        self.insert_hint = None;
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
        if let Some((edge, index)) = self.picked_reroute.take() {
            self.editor.remove_reroute(&edge, index);
            self.changed(cx);
            return;
        }
        let (nodes, marks) = self.split_selection();
        let frames = self.selected_frames();
        let edges: Vec<String> = self
            .selection
            .iter()
            .filter(|id| self.editor.graph().edges.iter().any(|edge| &edge.id == *id))
            .cloned()
            .collect();
        if nodes.is_empty() && marks.is_empty() && edges.is_empty() && frames.is_empty() {
            return;
        }
        self.editor.begin_gesture();
        for edge in &edges {
            self.editor.detach(edge);
        }
        // Deleting a frame removes the group only; its members stay.
        self.editor.delete_frames(&frames);
        self.editor.delete(&nodes, &marks);
        self.editor.commit_gesture();
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
        let handle = self.label_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.editing = Some(EditTarget::Mark(id));
        cx.notify();
    }

    /// Inline rename of a frame title. Enter commits, Escape cancels.
    pub fn start_frame_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self
            .editor
            .graph()
            .frames
            .iter()
            .find(|frame| frame.id == id)
        else {
            return;
        };
        let text = frame.title.clone();
        self.label_input.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        let handle = self.label_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.editing = Some(EditTarget::Frame(id.to_string()));
        cx.notify();
    }

    /// Commit the inline edit to whichever target is being renamed.
    fn commit_edit_target(
        &mut self,
        input: &Entity<gpui_kit::component::input::InputState>,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.editing.take() else {
            return;
        };
        let text = input.read(cx).value().to_string();
        match target {
            EditTarget::Mark(id) => self.editor.set_mark_text(&id, text),
            EditTarget::Frame(id) => self.editor.set_frame_title(&id, text),
        }
        self.changed(cx);
    }

    /// Escape pressed while renaming: drop the edit without writing.
    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            cx.notify();
        }
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

    pub fn set_note(&mut self, node: &str, note: impl Into<String>, cx: &mut Context<Self>) {
        let note = note.into();
        if note.is_empty() {
            self.notes.remove(node);
        } else {
            self.notes.insert(node.to_string(), note);
        }
        cx.notify();
    }

    pub fn commit_label(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_none() {
            return;
        }
        let text = self.label_input.read(cx).value().to_string();
        let Some(target) = self.editing.take() else {
            return;
        };
        match target {
            EditTarget::Mark(id) => self.editor.set_mark_text(&id, text),
            EditTarget::Frame(id) => self.editor.set_frame_title(&id, text),
        }
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
            from: Some((from.to_string(), from_port.to_string())),
            to: None,
            rewired: Vec::new(),
            pointer: at,
            origin: at,
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
            from: Some((from.to_string(), from_port.to_string())),
            to: None,
            rewired: Vec::new(),
            pointer: self.canvas_of(point(px(local.0), px(local.1))),
            origin: self.canvas_of(point(px(local.0), px(local.1))),
        });
        self.refresh_link_status();
        cx.notify();
    }

    /// Open the node search as if a wire from an output was dropped at `at`.
    /// Used by the screenshot reel.
    pub fn seed_search_drop(
        &mut self,
        from: &str,
        from_port: &str,
        at: Point,
        cx: &mut Context<Self>,
    ) {
        let ty = graph::output_types(self.editor.graph(), from)
            .into_iter()
            .next();
        let local = self.editor.graph().camera.to_window(at);
        let screen = self.screen_of(point(px(local.x), px(local.y)));
        cx.emit(CanvasEvent::OpenCatalog {
            from: Some((from.to_string(), from_port.to_string())),
            to: None,
            ty,
            at,
            screen,
        });
        cx.notify();
    }

    pub fn set_space_hand(&mut self, down: bool, cx: &mut Context<Self>) {
        self.space_hand = down;
        cx.notify();
    }

    fn select_hit(&mut self, local: GPoint<Pixels>) {
        let canvas = self.canvas_of(local);
        let hidden = crate::frames::collapsed_members(self.editor.graph());
        for node in self.editor.graph().nodes.iter().rev() {
            if hidden.contains_key(&node.id) {
                continue;
            }
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
        // Wire shapes derive from the document; an edit invalidates every
        // cached chain. Camera-only updates skip this and keep theirs.
        self.wire_cache.clear();
        cx.emit(CanvasEvent::Edited);
        cx.notify();
    }

    /// Public emit for menu handlers that live outside this module.
    pub fn notify_edited(&mut self, cx: &mut Context<Self>) {
        self.changed(cx);
    }

    /// Window-space bounds of the canvas panel, for popovers that must stay
    /// inside it.
    pub fn panel_bounds(&self) -> Option<Bounds<Pixels>> {
        *self.viewport.borrow()
    }

    /// Frame ids in the selection.
    fn selected_frames(&self) -> Vec<String> {
        self.selection
            .iter()
            .filter(|id| self.editor.graph().frames.iter().any(|f| f.id == **id))
            .cloned()
            .collect()
    }

    /// Group the selection with 24 px of padding around it.
    pub fn group_selection(&mut self, cx: &mut Context<Self>) {
        let (nodes, marks) = self.split_selection();
        if nodes.is_empty() && marks.is_empty() {
            return;
        }
        let Some(id) = self
            .editor
            .frame_around(&nodes, &marks, crate::frames::GROUP_PAD)
        else {
            return;
        };
        self.selection = vec![id.clone()];
        self.changed(cx);
        cx.emit(CanvasEvent::SelectionChanged);
    }

    /// Layered left-to-right layout of the selection, or of everything when
    /// fewer than two nodes are selected. One undo step, animated.
    pub fn auto_layout(&mut self, cx: &mut Context<Self>) {
        if self.editor.is_readonly() {
            return;
        }
        let (nodes, _) = self.split_selection();
        let ids: Vec<String> = if nodes.len() >= 2 {
            nodes
        } else {
            self.editor
                .graph()
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .collect()
        };
        if ids.len() < 2 {
            return;
        }
        let placed = crate::autolayout::layered_layout(self.editor.graph(), &ids);
        if placed.is_empty() {
            return;
        }
        let mut offsets = BTreeMap::new();
        for (id, next) in &placed {
            if let Some(node) = self.editor.graph().node(id) {
                offsets.insert(
                    id.clone(),
                    Point::new(node.position[0] - next.x, node.position[1] - next.y),
                );
            }
        }
        self.editor.place_many(&placed);
        if !self.reduced_motion
            && offsets
                .values()
                .any(|offset| offset.x.abs() + offset.y.abs() > 0.5)
        {
            self.settle = Some(Settle {
                offsets,
                started: Instant::now(),
                duration: motion::LAYOUT,
            });
        }
        // Fit the camera to the laid-out result.
        let view = self
            .viewport
            .borrow()
            .map(|bounds| (bounds.size.width.as_f32(), bounds.size.height.as_f32()));
        if let Some((w, h)) = view {
            let rect = graph::bounds_of(self.editor.graph(), &ids, card_height);
            if let Some(rect) = rect {
                self.animate_camera(graph::fit_camera(w, h, rect, 72.0), cx);
            }
        }
        self.changed(cx);
    }

    /// Flip the minimap on or off. The flag lives in the document view, so
    /// the next open remembers it; no undo step is written.
    pub fn toggle_minimap(&mut self, cx: &mut Context<Self>) {
        let mut view = self.editor.graph().view;
        view.minimap = !view.minimap;
        self.editor.set_view(view);
        self.minimap_fade = Some((view.minimap, Instant::now()));
        self.changed(cx);
    }

    /// Opacity of the minimap during its fade, 1 when steady.
    fn minimap_alpha(&self, showing: bool) -> f32 {
        let Some((target, started)) = self.minimap_fade else {
            return u8::from(showing) as f32;
        };
        let t = (started.elapsed().as_secs_f32() / motion::FADE.as_secs_f32()).clamp(0.0, 1.0);
        let t = 1.0 - (1.0 - t).powi(3);
        if target { t } else { 1.0 - t }
    }

    fn minimap_point(&self, event_position: GPoint<Pixels>) -> Option<Point> {
        let bounds = (*self.viewport.borrow())?;
        let local = event_position - bounds.origin;
        let view = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
        let content = crate::minimap::content_bounds(self.editor.graph(), card_height, view);
        let map = crate::minimap::Map::fit(content, crate::minimap::SIZE);
        let panel = Point::new(
            local.x.as_f32() - (view.0 - crate::minimap::SIZE[0] - crate::minimap::MARGIN),
            local.y.as_f32() - (view.1 - crate::minimap::SIZE[1] - crate::minimap::MARGIN),
        );
        Some(map.to_canvas(panel))
    }

    /// Click on the minimap: animate the camera so the point centers.
    pub fn minimap_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(canvas) = self.minimap_point(event.position) else {
            return;
        };
        self.minimap_drag = true;
        self.center_camera_on(canvas, true, cx);
    }

    /// Drag on the minimap: the camera follows the pointer immediately.
    pub fn minimap_drag_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.minimap_drag {
            return;
        }
        if let Some(canvas) = self.minimap_point(event.position) {
            self.center_camera_on(canvas, false, cx);
        }
    }

    /// Camera of the viewport centered on `at`, keeping the current zoom.
    fn center_camera_on(&mut self, at: Point, animate: bool, cx: &mut Context<Self>) {
        let Some(bounds) = *self.viewport.borrow() else {
            return;
        };
        let view = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
        let zoom = self.editor.graph().camera.zoom;
        let camera = Camera {
            x: view.0 * 0.5 - at.x * zoom,
            y: view.1 * 0.5 - at.y * zoom,
            zoom,
        };
        if animate {
            self.animate_camera(camera, cx);
        } else {
            self.camera_anim = None;
            self.editor.set_camera(camera);
            cx.emit(CanvasEvent::Edited);
            cx.notify();
        }
    }

    /// Press on a frame header: select the frame and open a move gesture.
    pub fn frame_header_down(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.editor.is_readonly() {
            return;
        }
        window.focus(&self.focus, cx);
        if !self.selection.iter().any(|item| item == id) {
            self.selection = vec![id.to_string()];
            cx.emit(CanvasEvent::SelectionChanged);
        }
        let Some(_frame) = self
            .editor
            .graph()
            .frames
            .iter()
            .find(|frame| frame.id == id)
        else {
            return;
        };
        let last = self.canvas_of(self.local_of(event.position));
        self.editor.begin_gesture();
        self.drag = Some(Drag::FrameMove {
            id: id.to_string(),
            last,
        });
        cx.notify();
    }

    /// Press on a resize handle: open a frame resize gesture.
    pub fn frame_resize_down(
        &mut self,
        id: &str,
        edge: crate::frames::ResizeEdge,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.editor.is_readonly() {
            return;
        }
        let start_size = self
            .editor
            .graph()
            .frames
            .iter()
            .find(|frame| frame.id == id)
            .map(|frame| frame.size);
        let Some(start_size) = start_size else {
            return;
        };
        let origin_local = self.local_of(event.position);
        self.editor.begin_gesture();
        self.drag = Some(Drag::FrameResize {
            id: id.to_string(),
            origin_local,
            start_size,
            edge,
        });
        cx.notify();
    }

    /// Collapse or expand a frame, animating the height.
    pub fn toggle_frame_collapsed(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(frame) = self
            .editor
            .graph()
            .frames
            .iter()
            .find(|frame| frame.id == id)
        else {
            return;
        };
        let target = !frame.collapsed;
        let from = self.frame_height(frame);
        self.editor.set_frame_collapsed(id, target);
        if !self.reduced_motion {
            self.frame_anim.insert(
                id.to_string(),
                FrameAnim {
                    from,
                    started: Instant::now(),
                },
            );
        } else {
            self.frame_anim.remove(id);
        }
        self.changed(cx);
    }

    /// Visible height of a frame, honoring the collapse animation.
    fn frame_height(&self, frame: &Frame) -> f32 {
        let target = if frame.collapsed {
            crate::frames::HEADER_H
        } else {
            frame.size[1]
        };
        if self.reduced_motion {
            return target;
        }
        match self.frame_anim.get(&frame.id) {
            Some(anim) => {
                let t = (anim.started.elapsed().as_secs_f32() / motion::COLLAPSE.as_secs_f32())
                    .clamp(0.0, 1.0);
                motion::lerp(anim.from, target, t)
            }
            None => target,
        }
    }

    /// Select every node the frame covers.
    pub fn select_frame_members(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selection = graph::frame_members(self.editor.graph(), id);
        cx.emit(CanvasEvent::SelectionChanged);
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
            let canvas = self.canvas_of(local);
            self.wire_pick = if self.selection.is_empty() {
                self.wire_hit_at(canvas).map(|(edge, segment)| {
                    self.selection = vec![edge.clone()];
                    (edge, canvas, segment)
                })
            } else {
                None
            };
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
        if self.tool == Tool::Frame {
            if self.editor.is_readonly() {
                return;
            }
            let canvas = self.canvas_of(local);
            self.editor.begin_gesture();
            self.drag = Some(Drag::FrameDraw {
                start: canvas,
                current: canvas,
            });
            cx.notify();
            return;
        }
        let canvas = self.canvas_of(local);
        // A double click adds a bend to a wire, removes a bend point, or
        // opens the node search on empty canvas.
        if event.click_count >= 2 {
            if let Some((edge, index)) = self.reroute_at(canvas) {
                self.picked_reroute = None;
                self.editor.remove_reroute(&edge, index);
                self.changed(cx);
                return;
            }
            if let Some((edge, segment)) = self.wire_hit_at(canvas) {
                if self.editor.is_readonly() {
                    return;
                }
                self.editor.begin_gesture();
                self.editor.add_reroute(&edge, segment, canvas);
                self.editor.commit_gesture();
                self.changed(cx);
                return;
            }
            if self.editor.is_readonly() {
                return;
            }
            let screen = self.screen_of(local);
            cx.emit(CanvasEvent::OpenCatalog {
                from: None,
                to: None,
                ty: None,
                at: canvas,
                screen,
            });
            cx.notify();
            return;
        }
        if let Some((edge, index)) = self.reroute_at(canvas) {
            self.picked_reroute = Some((edge.clone(), index));
            self.selection.clear();
            self.editor.begin_gesture();
            self.drag = Some(Drag::Reroute { edge, index });
            cx.emit(CanvasEvent::SelectionChanged);
            cx.notify();
            return;
        }
        if let Some((edge, _)) = self.wire_hit_at(canvas) {
            self.picked_reroute = None;
            self.selection = vec![edge.clone()];
            cx.emit(CanvasEvent::SelectionChanged);
            // A click on a wire shows the frame it carries. The workspace
            // turns this into a preview job for the wire's source node.
            if event.click_count == 1 {
                cx.emit(CanvasEvent::PreviewWire { edge });
            }
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
        let pointer = self.canvas_of(local);
        let origin = pointer;
        if side == Side::Out {
            self.editor.begin_gesture();
            let wires: Vec<Edge> = self
                .editor
                .graph()
                .edges
                .iter()
                .filter(|edge| edge.from == node && edge.from_port == port)
                .cloned()
                .collect();
            // Ctrl or Alt picks up every wire of the output instead of
            // starting one new wire.
            let pickup = event.modifiers.control || event.modifiers.alt;
            let mut rewired = Vec::new();
            if pickup {
                for edge in wires {
                    if let Some(detached) = self.editor.detach(&edge.id) {
                        rewired.push(detached);
                    }
                }
            }
            self.drag = Some(Drag::Link {
                from: Some((node.to_string(), port.to_string())),
                to: None,
                rewired,
                pointer,
                origin,
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
            if let Some(detached) = detached {
                self.drag = Some(Drag::Link {
                    from: Some((detached.from.clone(), detached.from_port.clone())),
                    to: Some((node.to_string(), port.to_string())),
                    rewired: Vec::new(),
                    pointer,
                    origin,
                });
            }
        } else {
            // A drag from an unconnected input port: the new wire keeps the
            // input as its anchor and reaches for an output.
            self.editor.begin_gesture();
            self.drag = Some(Drag::Link {
                from: None,
                to: Some((node.to_string(), port.to_string())),
                rewired: Vec::new(),
                pointer,
                origin,
            });
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
        self.insert_hint = None;
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
            self.track_hover(local, cx);
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
                // A single unconnected node near a wire can be inserted into it.
                self.insert_hint = None;
                if marks.is_empty() && nodes.len() == 1 {
                    let id = nodes.keys().next().cloned().unwrap_or_default();
                    self.update_insert_hint(&id, self.canvas_of(local));
                }
            }
            Drag::Link { pointer, .. } => {
                *pointer = self.canvas_of(local);
                refresh_link = true;
            }
            Drag::Reroute { edge, index } => {
                let at = self.canvas_of(local);
                // The snap-disable modifier is the one card drags use.
                let at = if event.modifiers.alt {
                    at
                } else {
                    graph::snap_grid(at)
                };
                self.editor.move_reroute(edge, *index, at);
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
            Drag::FrameDraw { current, .. } => {
                *current = self.canvas_of(local);
            }
            Drag::FrameMove { id, last } => {
                let at = self.canvas_of(local);
                let delta = Point::new(at.x - last.x, at.y - last.y);
                *last = at;
                self.editor.move_frame(id, delta);
            }
            Drag::FrameResize {
                id,
                origin_local,
                start_size,
                edge,
            } => {
                let zoom = self.editor.graph().camera.zoom.max(0.05);
                let dx = (local.x.as_f32() - origin_local.x.as_f32()) / zoom;
                let dy = (local.y.as_f32() - origin_local.y.as_f32()) / zoom;
                let mut size = *start_size;
                if *edge != crate::frames::ResizeEdge::Bottom {
                    size[0] = (start_size[0] + dx).max(24.0);
                }
                if *edge != crate::frames::ResizeEdge::Right {
                    size[1] = (start_size[1] + dy).max(24.0);
                }
                self.editor.resize_frame(id, size);
            }
        }
        self.drag = Some(drag);
        if refresh_link {
            self.refresh_link_status();
        }
        cx.notify();
    }

    /// Remember which wire and bend point sit under the pointer.
    fn track_hover(&mut self, local: GPoint<Pixels>, cx: &mut Context<Self>) {
        let canvas = self.canvas_of(local);
        let reroute = self.reroute_at(canvas);
        let wire = if reroute.is_none() {
            self.wire_hit_at(canvas)
        } else {
            None
        };
        if self.hover_reroute != reroute || self.hover_wire != wire {
            self.hover_reroute = reroute;
            self.hover_wire = wire;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let local = self.local_of(event.position);
        self.minimap_drag = false;
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
                // Dropping one unconnected node on a highlighted wire threads
                // it in; the move and the insert share the open gesture.
                if let Some(edge) = self.insert_hint.take()
                    && marks.is_empty()
                    && nodes.len() == 1
                    && let Some(id) = nodes.keys().next()
                    && self
                        .editor
                        .graph()
                        .edges
                        .iter()
                        .all(|wire| wire.from != *id && wire.to != *id)
                {
                    match self.editor.insert_node_on_edge(&edge, id) {
                        Ok(()) => {
                            self.selection = vec![id.clone()];
                            self.status = "node inserted into wire".to_string();
                            cx.emit(CanvasEvent::SelectionChanged);
                        }
                        Err(reason) => {
                            self.status = reject_message(reason, "node", "wire");
                        }
                    }
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
                to,
                rewired,
                pointer,
                origin,
                ..
            }) => {
                if !rewired.is_empty() {
                    let ids: Vec<String> = rewired.iter().map(|edge| edge.id.clone()).collect();
                    let origin = from.clone().unwrap_or_default().0;
                    if let Some((node, port)) = self.output_drop_at(pointer, &origin) {
                        let (moved, rejected) = self.editor.rewire_sources(&ids, &node, &port);
                        self.editor.commit_gesture();
                        self.status = if rejected > 0 {
                            format!("moved {moved}, rejected {rejected}")
                        } else if moved == 1 {
                            "moved 1 wire".to_string()
                        } else {
                            format!("moved {moved} wires")
                        };
                    } else {
                        // Empty space cancels: the gesture restores the wires.
                        self.editor.cancel_gesture();
                        self.status.clear();
                    }
                    self.changed(cx);
                } else if let Some((from_node, from_port)) = from.clone()
                    && let Some((to_node, to_port)) = self.snap_at(pointer, &from_node, &from_port)
                {
                    let _ = self
                        .editor
                        .connect(&from_node, &from_port, &to_node, &to_port);
                    self.status.clear();
                    self.editor.commit_gesture();
                    self.changed(cx);
                } else if from.is_none()
                    && let Some((to_node, to_port)) = to.clone()
                    && let Some((node, port)) = self.output_drop_at(pointer, "")
                {
                    let _ = self.editor.connect(&node, &port, &to_node, &to_port);
                    self.status.clear();
                    self.editor.commit_gesture();
                    self.changed(cx);
                } else if let Some((to_node, to_port)) = to.clone() {
                    // An input end was dropped on empty space: offer nodes
                    // that can feed the input again.
                    self.status.clear();
                    let ty = self
                        .editor
                        .graph()
                        .node(&to_node)
                        .and_then(|node| {
                            node.input_ports()
                                .into_iter()
                                .find(|slot| slot.name == to_port)
                        })
                        .and_then(|slot| slot.accepts.first().copied());
                    let screen = self.screen_of(local);
                    cx.emit(CanvasEvent::OpenCatalog {
                        from: None,
                        to: Some((to_node, to_port)),
                        ty,
                        at: pointer,
                        screen,
                    });
                    self.editor.commit_gesture();
                    self.changed(cx);
                } else if let Some((from_node, _)) = from.clone()
                    // A click on the output port that never dragged: show the
                    // frame this output produces. A real drag a few pixels
                    // away still opens the catalog.
                    && (pointer.x - origin.x).abs() + (pointer.y - origin.y).abs() < 6.0 / self.zoom()
                {
                    self.status.clear();
                    self.editor.commit_gesture();
                    self.selection = vec![from_node.clone()];
                    cx.emit(CanvasEvent::SelectionChanged);
                    cx.emit(CanvasEvent::PreviewPort { node: from_node });
                    cx.notify();
                } else if let Some((from_node, from_port)) = from.clone() {
                    self.status.clear();
                    let ty = graph::output_types(self.editor.graph(), &from_node)
                        .into_iter()
                        .next();
                    let screen = self.screen_of(local);
                    cx.emit(CanvasEvent::OpenCatalog {
                        from: Some((from_node, from_port)),
                        to: None,
                        ty,
                        at: pointer,
                        screen,
                    });
                    self.editor.commit_gesture();
                    self.changed(cx);
                } else {
                    self.editor.commit_gesture();
                    self.changed(cx);
                }
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
                    let hidden = crate::frames::collapsed_members(self.editor.graph());
                    let mut hits = Vec::new();
                    for node in &self.editor.graph().nodes {
                        if hidden.contains_key(&node.id) {
                            continue;
                        }
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
            Some(Drag::Reroute { .. }) => {
                self.editor.commit_gesture();
                self.changed(cx);
            }
            Some(Drag::Resize { .. }) => {
                self.editor.commit_gesture();
                self.changed(cx);
            }
            Some(Drag::FrameDraw { start, current }) => {
                self.editor.cancel_gesture();
                let x = start.x.min(current.x);
                let y = start.y.min(current.y);
                let w = (start.x - current.x).abs().max(0.0);
                let h = (start.y - current.y).abs().max(0.0);
                if w >= 24.0
                    && h >= 24.0
                    && let Some(id) = self
                        .editor
                        .add_frame(String::new(), Point::new(x, y), [w, h])
                {
                    self.selection = vec![id];
                    cx.emit(CanvasEvent::SelectionChanged);
                }
                self.changed(cx);
            }
            Some(Drag::FrameMove { .. }) => {
                self.editor.commit_gesture();
                self.changed(cx);
            }
            Some(Drag::FrameResize { .. }) => {
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
        let hidden = crate::frames::collapsed_members(self.editor.graph());
        let mut candidates = Vec::new();
        for node in &self.editor.graph().nodes {
            if node.id == from || hidden.contains_key(&node.id) {
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
            to,
            pointer,
            rewired,
            ..
        }) = &self.drag
        else {
            return;
        };
        let from = from.clone();
        let to = to.clone();
        let pointer = *pointer;
        let rewired = rewired.len();
        if rewired > 0 {
            self.status = format!("moving {rewired} wires");
            return;
        }
        if let Some((from_node, from_port)) = from {
            let snap = self.snap_at(pointer, &from_node, &from_port);
            if let Some((to_node, to_port)) = snap {
                let (left, right) =
                    type_pair_label(self.editor.graph(), &from_node, &to_node, &to_port);
                self.status = format!("{} → {}", shorten_types(&left), shorten_types(&right));
                return;
            }
            if let Some((node, port, reason)) = self.nearest_reject(pointer, &from_node, &from_port)
            {
                let (left, right) = type_pair_label(self.editor.graph(), &from_node, &node, &port);
                self.status = reject_message(reason, &shorten_types(&left), &shorten_types(&right));
            } else {
                self.status.clear();
            }
            return;
        }
        if let Some((to_node, to_port)) = to
            && let Some((node, _)) = self.output_drop_at(pointer, "")
        {
            let (left, right) = type_pair_label(self.editor.graph(), &node, &to_node, &to_port);
            self.status = format!("{} → {}", shorten_types(&left), shorten_types(&right));
            return;
        }
        self.status.clear();
    }

    fn nearest_reject(
        &self,
        pointer: Point,
        from: &str,
        from_port: &str,
    ) -> Option<(String, String, Reject)> {
        let zoom = self.editor.graph().camera.zoom.max(0.05);
        let hidden = crate::frames::collapsed_members(self.editor.graph());
        let mut best: Option<(f32, String, String, Reject)> = None;
        for node in &self.editor.graph().nodes {
            if node.id == from || hidden.contains_key(&node.id) {
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
            || !self.frame_anim.is_empty()
            || self.minimap_fade.is_some()
            || (self.badges.values().any(|badge| *badge == Badge::Running) && !self.reduced_motion)
    }

    fn tick_motion(&mut self) {
        if let Some(settle) = &self.settle
            && settle.started.elapsed() >= settle.duration
        {
            self.settle = None;
        }
        self.frame_anim
            .retain(|_, anim| anim.started.elapsed() < motion::COLLAPSE);
        if let Some((_, started)) = &self.minimap_fade
            && started.elapsed() >= motion::FADE
        {
            self.minimap_fade = None;
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
        self.badge_fade
            .retain(|_, (_, started)| started.elapsed() < BADGE_FADE);
        if self.badges.values().any(|badge| *badge == Badge::Running) && !self.reduced_motion {
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
            to,
            pointer,
            rewired,
            ..
        } = self.drag.as_ref()?
        else {
            return None;
        };
        if !rewired.is_empty() {
            // Every picked wire draws from its own source port to the pointer.
            let mut starts = Vec::new();
            for edge in rewired {
                let source = self.editor.graph().node(&edge.from)?;
                let slot = port_slots(source)
                    .into_iter()
                    .find(|slot| slot.name == edge.from_port && slot.side == Side::Out)?;
                starts.push(slot.center);
            }
            let first = *starts.first()?;
            return Some(LinkVisual {
                from: first,
                to: *pointer,
                extra_from: starts[1..].to_vec(),
                snapped: None,
                reject: None,
                message: self.status.clone(),
            });
        }
        if let Some((from_node, from_port)) = from {
            let source = self.editor.graph().node(from_node.as_str())?;
            let slot = port_slots(source)
                .into_iter()
                .find(|slot| slot.name == from_port.as_str() && slot.side == Side::Out)?;
            let snap = self.snap_at(*pointer, from_node, from_port);
            let reject = if snap.is_none() {
                self.nearest_reject(*pointer, from_node, from_port)
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
            return Some(LinkVisual {
                from: slot.center,
                to: end,
                extra_from: Vec::new(),
                snapped: snap,
                reject: reject.map(|(_, _, reason)| reason),
                message: self.status.clone(),
            });
        }
        // A drag from an input side: the wire hangs from the pointer and
        // reaches for an output.
        let (to_node, to_port) = to.as_ref()?;
        let target = self.editor.graph().node(to_node)?;
        let slot = port_slots(target)
            .into_iter()
            .find(|slot| slot.name == *to_port && slot.side == Side::In)?;
        let snap = self.output_drop_at(*pointer, "");
        let end = if let Some((node, port)) = &snap {
            self.editor
                .graph()
                .node(node)
                .and_then(|node| {
                    port_slots(node)
                        .into_iter()
                        .find(|slot| slot.name == *port && slot.side == Side::Out)
                        .map(|slot| slot.center)
                })
                .unwrap_or(*pointer)
        } else {
            *pointer
        };
        Some(LinkVisual {
            from: end,
            to: slot.center,
            extra_from: Vec::new(),
            snapped: snap,
            reject: None,
            message: self.status.clone(),
        })
    }

    /// Window position of a canvas-local point, for popovers that anchor to
    /// the pointer.
    fn screen_of(&self, local: GPoint<Pixels>) -> GPoint<Pixels> {
        let origin = self
            .viewport
            .borrow()
            .map(|bounds| bounds.origin)
            .unwrap_or_else(|| point(px(0.0), px(0.0)));
        origin + local
    }

    fn zoom(&self) -> f32 {
        self.editor.graph().camera.zoom.max(0.05)
    }

    /// Nearest wire segment within 8 screen px, as edge id and segment index.
    /// Wires route through collapsed frame headers, and wires with both ends
    /// hidden do not hit at all.
    fn wire_hit_at(&self, canvas: Point) -> Option<(String, usize)> {
        let graph = self.editor.graph();
        let hidden = crate::frames::collapsed_members(graph);
        let radius = 8.0 / self.zoom();
        for edge in graph.edges.iter().rev() {
            if let Some(path) =
                crate::frames::edge_wire_path_routed(&graph.nodes, &graph.frames, &hidden, edge)
                && let Some(segment) = layout::wire_hit(&path, canvas, radius)
            {
                return Some((edge.id.clone(), segment));
            }
        }
        None
    }

    /// Nearest reroute point within 7 screen px, as edge id and point index.
    fn reroute_at(&self, canvas: Point) -> Option<(String, usize)> {
        let graph = self.editor.graph();
        let radius = 7.0 / self.zoom();
        let mut best: Option<(f32, String, usize)> = None;
        for edge in graph.edges.iter().rev() {
            for (index, point) in edge.reroutes.iter().enumerate() {
                let dx = point[0] - canvas.x;
                let dy = point[1] - canvas.y;
                let distance = dx * dx + dy * dy;
                if distance <= radius * radius
                    && best.as_ref().is_none_or(|(top, _, _)| distance < *top)
                {
                    best = Some((distance, edge.id.clone(), index));
                }
            }
        }
        best.map(|(_, edge, index)| (edge, index))
    }

    /// Nearest output port within the snap radius. `origin` excludes the port
    /// the drag started from; an empty origin means every output counts.
    fn output_drop_at(&self, pointer: Point, origin: &str) -> Option<(String, String)> {
        let graph = self.editor.graph();
        let zoom = self.zoom();
        let hidden = crate::frames::collapsed_members(graph);
        let origin_types = if origin.is_empty() {
            Vec::new()
        } else {
            graph::output_types(graph, origin)
        };
        let mut candidates = Vec::new();
        for node in &graph.nodes {
            if node.id == origin || hidden.contains_key(&node.id) {
                continue;
            }
            for slot in port_slots(node) {
                if slot.side != Side::Out {
                    continue;
                }
                let dx = (slot.center.x - pointer.x) * zoom;
                let dy = (slot.center.y - pointer.y) * zoom;
                let distance = (dx * dx + dy * dy).sqrt();
                let types = graph::output_types(graph, &node.id);
                let compatible = if origin_types.is_empty() {
                    !types.is_empty()
                } else {
                    types.iter().any(|ty| origin_types.contains(ty))
                };
                candidates.push(SnapCandidate {
                    node: node.id.clone(),
                    port: slot.name.clone(),
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

    /// The wire a dragged unconnected node could be inserted into, when the
    /// pointer is within 8 screen px and both connect rules would accept it.
    fn update_insert_hint(&mut self, node_id: &str, pointer: Point) {
        self.insert_hint = None;
        let graph = self.editor.graph();
        let Some(node) = graph.node(node_id) else {
            return;
        };
        if graph
            .edges
            .iter()
            .any(|edge| edge.from == node_id || edge.to == node_id)
        {
            return;
        }
        let Some(input) = node.input_ports().into_iter().next() else {
            return;
        };
        let Some(output) = node.output_port() else {
            return;
        };
        let radius = 8.0 / self.zoom();
        let hidden = crate::frames::collapsed_members(graph);
        for edge in &graph.edges {
            if edge.from == node_id || edge.to == node_id {
                continue;
            }
            let Some(path) =
                crate::frames::edge_wire_path_routed(&graph.nodes, &graph.frames, &hidden, edge)
            else {
                continue;
            };
            if layout::wire_hit(&path, pointer, radius).is_none() {
                continue;
            }
            let mut trial = graph.clone();
            trial.edges.retain(|item| item.id != edge.id);
            if try_connect(&trial, &edge.from, &edge.from_port, node_id, &input.name).is_ok()
                && try_connect(&trial, node_id, &output.name, &edge.to, &edge.to_port).is_ok()
            {
                self.insert_hint = Some(edge.id.clone());
                return;
            }
        }
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
    /// More source ports when several wires are picked up at once.
    extra_from: Vec<Point>,
    snapped: Option<(String, String)>,
    reject: Option<Reject>,
    message: String,
}

/// What the wire layer needs to know about hover, selection, and the drop
/// hint, beyond the selection itself.
#[derive(Clone, Default)]
struct WireOverlay {
    hover: Option<String>,
    hover_point: Option<(String, usize)>,
    picked: Option<(String, usize)>,
    hint: Option<String>,
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
                from: Some((node, port)),
                ..
            }) => Some((node.clone(), port.clone())),
            _ => None,
        };
        let overlay = WireOverlay {
            hover: self.hover_wire.as_ref().map(|(id, _)| id.clone()),
            hover_point: self.hover_reroute.clone(),
            picked: self.picked_reroute.clone(),
            hint: self.insert_hint.clone(),
        };
        let snap_target = link.as_ref().and_then(|item| item.snapped.clone());
        let flow =
            if self.badges.values().any(|badge| *badge == Badge::Running) && !self.reduced_motion {
                Some(self.flow)
            } else {
                None
            };
        let graph = self.editor.graph().clone();
        let hovered = self.hovered.clone();
        let hidden = crate::frames::collapsed_members(&graph);
        // Visible slice of the viewport, in element window space. Culling is
        // a render-time optimization: selection and gestures stay model based.
        let view_rect = self.viewport.borrow().map(|bounds| graph::Rect {
            x: 0.0,
            y: 0.0,
            w: bounds.size.width.as_f32(),
            h: bounds.size.height.as_f32(),
        });
        let placed = shifted(&nodes, &settle_ids);
        // Probe graph with the drawn positions so the pure culling helpers
        // see exactly what the paint will see, settle offsets included.
        let placed_graph = graph::Graph {
            nodes: placed,
            ..graph::Graph::default()
        };
        let card_on = if view_rect.is_some() {
            let mut flags = vec![false; nodes.len()];
            for index in crate::perf::visible_nodes(
                &placed_graph,
                &camera,
                view_rect.unwrap_or(graph::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                }),
                crate::perf::MARGIN_PX,
            ) {
                flags[index] = true;
            }
            flags
        } else {
            vec![true; nodes.len()]
        };
        let wire_paints: Vec<WirePaint> = {
            let mut cache = std::mem::take(&mut self.wire_cache);
            let mut paints: Vec<WirePaint> = Vec::with_capacity(edges.len());
            build_wire_paints(
                &placed_graph.nodes,
                &graph,
                &hidden,
                &edges,
                camera,
                view_rect,
                &chart,
                theme_muted,
                &mut cache,
                &mut paints,
            );
            self.wire_cache = cache;
            paints
        };
        let frame_paints: Vec<crate::frames::FramePaint> = graph
            .frames
            .iter()
            .map(|frame| crate::frames::FramePaint {
                color: crate::frames::swatch_color(&frame.swatch, &chart, theme_muted),
                height: self.frame_height(frame),
                selected: selection.iter().any(|id| id == &frame.id),
                members: graph::frame_members(&graph, &frame.id).len(),
                frame: frame.clone(),
            })
            .collect();
        let frame_draw = match &self.drag {
            Some(Drag::FrameDraw { start, current }) => Some((*start, *current)),
            _ => None,
        };
        let minimap_on = self.editor.graph().view.minimap;
        let minimap_alpha = self.minimap_alpha(minimap_on);
        let _minimap_animating = self.minimap_fade.is_some();

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
                } else if platform && key == "g" {
                    this.group_selection(cx);
                } else if !platform
                    && event.keystroke.modifiers.shift
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && key == "l"
                {
                    this.auto_layout(cx);
                } else if !platform
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    // Tool shortcuts. Plain letters, so a paste chord never reaches them.
                    let next = match key {
                        "v" => Some(Tool::Select),
                        "h" => Some(Tool::Hand),
                        "r" => Some(Tool::Rectangle),
                        "o" => Some(Tool::Circle),
                        "t" => Some(Tool::Label),
                        "n" => Some(Tool::Note),
                        "f" => Some(Tool::Frame),
                        _ => None,
                    };
                    if let Some(next) = next {
                        this.tool = next;
                        cx.notify();
                    } else if key == "m" {
                        this.toggle_minimap(cx);
                    } else if key == "escape" {
                        this.cancel_edit(cx);
                    }
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
                let painted_link = link.clone();
                let painted_overlay = overlay.clone();
                let painted_placed = placed_graph.nodes;
                let painted_wires = wire_paints;
                canvas(
                    move |bounds, _, _| {
                        *viewport.borrow_mut() = Some(bounds);
                    },
                    move |bounds, _, window, cx| {
                        paint_scene(
                            bounds,
                            camera,
                            &painted_placed,
                            &painted_selection,
                            painted_link.as_ref(),
                            flow,
                            theme_muted,
                            theme_primary,
                            theme_danger,
                            theme_border,
                            &painted_overlay,
                            &painted_chart,
                            &painted_wires,
                            window,
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full()
            })
            // Frame layers sit above the wires but below the cards, so a
            // card overlapping a frame stays on top. Off-screen frames are
            // culled entirely, members included.
            .children(
                frame_paints
                    .iter()
                    .filter(|paint| {
                        let at = camera.to_window(Point::new(
                            paint.frame.position[0],
                            paint.frame.position[1],
                        ));
                        let rect = graph::Rect {
                            x: at.x,
                            y: at.y,
                            w: paint.frame.size[0] * camera.zoom,
                            h: paint.height * camera.zoom,
                        };
                        view_rect.is_none_or(|view| {
                            crate::perf::rects_intersect(
                                rect,
                                crate::perf::grow(view, crate::perf::MARGIN_PX),
                            )
                        })
                    })
                    .map(|paint| {
                        crate::frames::frame_element(
                            entity.clone(),
                            paint.clone(),
                            crate::frames::FrameTheme {
                                fg: theme_fg,
                                muted: theme_muted,
                                accent: theme_primary,
                                card: theme_raised,
                            },
                            camera,
                            paint.frame.collapsed,
                        )
                    }),
            )
            .children(
                self.editor
                    .graph()
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(index, node)| card_on[*index] && !hidden.contains_key(&node.id))
                    .map(|(_, node)| {
                        let offset = settle_ids
                            .iter()
                            .find(|(id, _)| id == &node.id)
                            .map(|(_, offset)| *offset)
                            .unwrap_or_else(|| Point::new(0.0, 0.0));
                        let badge = self
                            .badges
                            .get(&node.id)
                            .cloned()
                            .unwrap_or(Badge::Configured);
                        let fade = self.badge_fade.get(&node.id).and_then(|(from, started)| {
                            if self.reduced_motion {
                                return None;
                            }
                            let t = started.elapsed().as_secs_f32() / BADGE_FADE.as_secs_f32();
                            (t < 1.0).then(|| {
                                let from = badge_color(
                                    from,
                                    theme_muted,
                                    theme_primary,
                                    theme_success,
                                    theme_warning,
                                    theme_danger,
                                );
                                (from, t)
                            })
                        });
                        card_element(
                            entity.clone(),
                            node,
                            &graph,
                            &badge,
                            fade,
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
                                note: self.notes.get(&node.id).cloned(),
                            },
                        )
                    }),
            )
            .children(
                mark_views
                    .iter()
                    .filter(|mark| {
                        let size = mark.size.unwrap_or(match mark.kind {
                            MarkKind::Label | MarkKind::Note | MarkKind::Button => [168.0, 36.0],
                            MarkKind::Rectangle | MarkKind::Circle => [140.0, 88.0],
                        });
                        let at = camera.to_window(Point::new(mark.position[0], mark.position[1]));
                        let rect = graph::Rect {
                            x: at.x,
                            y: at.y,
                            w: size[0] * camera.zoom,
                            h: size[1] * camera.zoom,
                        };
                        view_rect.is_none_or(|view| {
                            crate::perf::rects_intersect(
                                rect,
                                crate::perf::grow(view, crate::perf::MARGIN_PX),
                            )
                        })
                    })
                    .map(|mark| {
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
                    }),
            )
            .when_some(self.editing.clone(), |this, target| {
                let (at, width) = match &target {
                    EditTarget::Mark(id) => {
                        let Some(mark) = self.editor.graph().mark(id) else {
                            return this;
                        };
                        let at = camera.to_window(Point::new(mark.position[0], mark.position[1]));
                        let width = mark.size.map(|size| size[0]).unwrap_or(168.0) * camera.zoom;
                        (at, width)
                    }
                    EditTarget::Frame(id) => {
                        let Some(frame) = self
                            .editor
                            .graph()
                            .frames
                            .iter()
                            .find(|frame| frame.id == *id)
                        else {
                            return this;
                        };
                        let at = camera.to_window(Point::new(frame.position[0], frame.position[1]));
                        let width = (frame.size[0] * camera.zoom).max(px(160.0).as_f32());
                        (Point::new(at.x + 4.0, at.y + 4.0), (width - 8.0).max(120.0))
                    }
                };
                // Escape drops the edit. The catcher registers a capture
                // phase key listener for as long as the overlay paints, so
                // Escape is seen before the focused input.
                let cancel = cx.entity().downgrade();
                let escape_catcher = gpui_kit::canvas(
                    move |_, _, _| {},
                    move |_, _, window, _| {
                        window.on_key_event(
                            move |event: &gpui_kit::KeyDownEvent,
                                  phase: gpui_kit::DispatchPhase,
                                  _window,
                                  cx| {
                                if phase != gpui_kit::DispatchPhase::Capture {
                                    return;
                                }
                                if event.keystroke.key == "escape" {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.cancel_edit(cx);
                                    });
                                }
                            },
                        );
                    },
                )
                .absolute()
                .size_full();
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
                .child(escape_catcher)
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
            .when_some(frame_draw, |this, (start, current)| {
                let at =
                    camera.to_window(Point::new(start.x.min(current.x), start.y.min(current.y)));
                let w = (start.x - current.x).abs() * camera.zoom;
                let h = (start.y - current.y).abs() * camera.zoom;
                this.child(
                    div()
                        .absolute()
                        .left(px(at.x))
                        .top(px(at.y))
                        .w(px(w))
                        .h(px(h))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme_muted.opacity(0.35))
                        .bg(theme_muted.opacity(0.06)),
                )
            })
            .when(minimap_on || minimap_alpha > 0.001, |this| {
                let Some(bounds) = *self.viewport.borrow() else {
                    return this;
                };
                let view = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
                let content = crate::minimap::content_bounds(&graph, card_height, view);
                let map = crate::minimap::Map::fit(content, crate::minimap::SIZE);
                let entity = entity.clone();
                let graph = graph.clone();
                let selection = selection.clone();
                this.child(crate::minimap::minimap_element(
                    entity,
                    &graph,
                    &selection,
                    map,
                    minimap_alpha,
                    view,
                    cx,
                ))
            })
            .context_menu({
                let entity = cx.entity();
                move |menu, _, cx| canvas_menu(menu, entity.clone(), cx)
            })
    }
}

fn canvas_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: Entity<CanvasModel>,
    cx: &App,
) -> gpui_kit::component::menu::PopupMenu {
    let paste = entity.clone();
    let duplicate = entity.clone();
    let delete = entity.clone();
    let align = entity.clone();
    let front = entity.clone();
    let back = entity.clone();
    let edit = entity.clone();
    let mut menu = menu
        .item(PopupMenuItem::new("Paste").on_click(move |_, _, cx| {
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
        );
    let group = entity.clone();
    let layout = entity.clone();
    menu = menu
        .separator()
        .item(PopupMenuItem::new("Group").on_click(move |_, _, cx| {
            group.update(cx, |this, cx| this.group_selection(cx));
        }))
        .item(PopupMenuItem::new("Auto-layout").on_click(move |_, _, cx| {
            layout.update(cx, |this, cx| this.auto_layout(cx));
        }));
    // A right click on a card leaves its node selected. The execution
    // commands then land next to the edit ones.
    let node = entity.read(cx).selection.iter().find_map(|id| {
        entity
            .read(cx)
            .editor()
            .graph()
            .node(id)
            .map(|node| node.id.clone())
    });
    if let Some(node) = node {
        let run = entity.clone();
        let run_to = entity.clone();
        let reset = entity.clone();
        let run_node = node.clone();
        let run_to_node = node.clone();
        menu = menu
            .separator()
            .item(PopupMenuItem::new("Execute").on_click(move |_, _, cx| {
                run.update(cx, |_, cx| {
                    cx.emit(CanvasEvent::NodeCommand {
                        id: run_node.clone(),
                        command: NodeCommand::Execute,
                    })
                });
            }))
            .item(
                PopupMenuItem::new("Execute to here").on_click(move |_, _, cx| {
                    run_to.update(cx, |_, cx| {
                        cx.emit(CanvasEvent::NodeCommand {
                            id: run_to_node.clone(),
                            command: NodeCommand::ExecuteTo,
                        })
                    });
                }),
            )
            .item(PopupMenuItem::new("Reset").on_click(move |_, _, cx| {
                reset.update(cx, |_, cx| {
                    cx.emit(CanvasEvent::NodeCommand {
                        id: node.clone(),
                        command: NodeCommand::Reset,
                    })
                });
            }));
    }
    // A right click on a wire adds bend commands to the same menu.
    if let Some((edge, at, segment)) = entity.read(cx).wire_pick.clone() {
        let add = entity.clone();
        let clear = entity.clone();
        let clear_edge = edge.clone();
        menu = menu
            .separator()
            .item(
                PopupMenuItem::new("Add reroute point").on_click(move |_, _, cx| {
                    add.update(cx, |this, cx| {
                        this.editor.begin_gesture();
                        this.editor.add_reroute(&edge, segment, at);
                        this.editor.commit_gesture();
                        this.changed(cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new("Remove reroute points").on_click(move |_, _, cx| {
                    clear.update(cx, |this, cx| {
                        this.editor.clear_reroutes(&clear_edge);
                        this.changed(cx);
                    });
                }),
            );
    }
    menu
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
    note: Option<String>,
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
    selection: &[String],
    link: Option<&LinkVisual>,
    flow: Option<f32>,
    muted: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    overlay: &WireOverlay,
    chart: &[gpui_kit::Hsla; 6],
    wires: &[WirePaint],
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

    // Wires paint behind the frames, cards, and marks. Frames arrive as
    // translucent element layers above this scene.
    for wire in wires {
        let selected = selection.iter().any(|id| id == &wire.id);
        let hovered = overlay.hover.as_deref() == Some(wire.id.as_str());
        let hinted = overlay.hint.as_deref() == Some(wire.id.as_str());
        let base = wire.source_color.opacity(if selected { 1.0 } else { 0.85 });
        let (color, width) = if hinted {
            (primary, 2.5)
        } else if selected {
            (wire.source_color, 1.75)
        } else {
            (base, 1.25)
        };
        for cubic in wire.path.iter().copied() {
            paint_cubic(origin, camera, cubic, color, px(width), window);
        }
        if let Some(phase) = flow {
            let at = layout::cubic_point(wire.path[0], phase);
            paint_dot(origin, camera, at, 3.0, color, window);
        }
        // Bend points show when they matter: near the camera, or when the
        // wire or one of the points is being looked at.
        if wire.reroutes.is_empty() {
            continue;
        }
        let is_picked = |index: usize| {
            overlay
                .picked
                .as_ref()
                .is_some_and(|(id, i)| id == &wire.id && *i == index)
        };
        let is_hover_point = |index: usize| {
            overlay
                .hover_point
                .as_ref()
                .is_some_and(|(id, i)| id == &wire.id && *i == index)
        };
        for (index, point) in wire.reroutes.iter().copied().enumerate() {
            let active = selected || hovered || is_picked(index) || is_hover_point(index);
            if zoom < 0.55 && !active {
                continue;
            }
            let accent = is_picked(index) || is_hover_point(index);
            paint_dot(
                origin,
                camera,
                point,
                3.5,
                if accent { primary } else { border },
                window,
            );
        }
    }
    if let Some(link) = link {
        let mut starts = vec![link.from];
        starts.extend(link.extra_from.iter().copied());
        let color = if link.reject.is_some() {
            danger
        } else if link.snapped.is_some() || !link.extra_from.is_empty() {
            link_source_color(nodes, link, chart, primary)
        } else {
            muted
        };
        for start in starts {
            let cubic = layout::wire_cubic(start, link.to);
            paint_cubic(origin, camera, cubic, color, px(1.5), window);
        }
    }
}

/// Resolve the visible wires for the next paint. Each edge gets its chain
/// from the cache when the key still matches, so panning and zooming only
/// re-project; an edit clears the cache and everything rebuilds once. Wires
/// whose control box falls outside the viewport plus the margin are skipped,
/// but their cache entries survive so panning them back in is free.
#[allow(clippy::too_many_arguments)]
fn build_wire_paints(
    placed: &[Node],
    graph: &graph::Graph,
    hidden: &std::collections::BTreeMap<String, String>,
    edges: &[Edge],
    camera: Camera,
    viewport: Option<graph::Rect>,
    chart: &[gpui_kit::Hsla; 6],
    muted: gpui_kit::Hsla,
    cache: &mut HashMap<String, CachedWire>,
    out: &mut Vec<WirePaint>,
) {
    let view = viewport.map(|rect| crate::perf::grow(rect, crate::perf::MARGIN_PX));
    for edge in edges {
        let Some((from, to)) = crate::frames::edge_endpoints(placed, &graph.frames, hidden, edge)
        else {
            continue;
        };
        let reroutes: Vec<Point> = edge
            .reroutes
            .iter()
            .map(|point| Point::new(point[0], point[1]))
            .collect();
        let key = WireKey { from, to, reroutes };
        let chain = match cache.get(&edge.id) {
            Some(cached) if cached.key == key => cached.path.clone(),
            _ => {
                let path = Arc::new(layout::wire_path(from, &key.reroutes, to));
                cache.insert(
                    edge.id.clone(),
                    CachedWire {
                        key: key.clone(),
                        path: path.clone(),
                    },
                );
                path
            }
        };
        let rect = crate::perf::chain_window_box(&chain, camera);
        let Some(view) = view else {
            out.push(wire_paint(edge, chain, key, graph, chart, muted));
            continue;
        };
        if !crate::perf::rects_intersect(rect, view) {
            continue;
        }
        out.push(wire_paint(edge, chain, key, graph, chart, muted));
    }
}

fn wire_paint(
    edge: &Edge,
    chain: Arc<Vec<layout::Cubic>>,
    key: WireKey,
    graph: &graph::Graph,
    chart: &[gpui_kit::Hsla; 6],
    muted: gpui_kit::Hsla,
) -> WirePaint {
    let source_color = output_types(graph, &edge.from)
        .into_iter()
        .next()
        .map(|ty| type_color(ty, chart))
        .unwrap_or(muted);
    WirePaint {
        id: edge.id.clone(),
        reroutes: key.reroutes,
        path: chain,
        source_color,
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

/// Fill color of a badge. The hollow ring uses the border color instead,
/// which `card_element` handles.
fn badge_color(
    badge: &Badge,
    muted: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
    success: gpui_kit::Hsla,
    warning: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
) -> gpui_kit::Hsla {
    match badge {
        Badge::NotConfigured(_) | Badge::Configured | Badge::Inactive => muted.opacity(0.55),
        Badge::Running => primary,
        Badge::Executed => success,
        Badge::Error(_) => danger,
        Badge::Stale => warning,
    }
}

/// Linear blend of two theme colors, eased with the shared curve.
fn mix_color(from: gpui_kit::Hsla, to: gpui_kit::Hsla, t: f32) -> gpui_kit::Hsla {
    let t = motion::ease_out(t);
    gpui_kit::Hsla {
        h: from.h + (to.h - from.h) * t,
        s: from.s + (to.s - from.s) * t,
        l: from.l + (to.l - from.l) * t,
        a: from.a + (to.a - from.a) * t,
    }
}

/// The badge that sits at the end of a card header. Shows a tooltip with the
/// state the canvas is claiming for the node.
fn badge_element(
    node_id: &str,
    badge: &Badge,
    color: gpui_kit::Hsla,
    ring: gpui_kit::Hsla,
    zoom: f32,
) -> gpui_kit::AnyElement {
    let size = px(7.0 * zoom);
    let id = SharedString::from(format!("badge-{node_id}"));
    let text = badge.tooltip_text();
    let base = div()
        .id(id)
        .size(size)
        .flex_shrink_0()
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
        });
    if matches!(badge, Badge::NotConfigured(_)) {
        base.rounded_full()
            .border_1()
            .border_color(ring)
            .into_any_element()
    } else {
        base.rounded_full().bg(color).into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn card_element(
    entity: Entity<CanvasModel>,
    node: &Node,
    graph: &graph::Graph,
    badge: &Badge,
    fade: Option<(gpui_kit::Hsla, f32)>,
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
    let detail = layout::detail_for(zoom);
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
    let ring = matches!(badge, Badge::NotConfigured(_));
    let pulse = if matches!(badge, Badge::Running) && !reduced_motion {
        let wave = (Instant::now().elapsed().as_secs_f32() * 4.0).sin().abs();
        0.45 + 0.55 * wave
    } else {
        1.0
    };
    let target = badge_color(
        badge,
        paint.muted,
        paint.primary,
        paint.success,
        paint.warning,
        paint.danger,
    )
    .opacity(pulse);
    // A badge color takes 120 ms to settle into the next one. Reduced motion
    // (and a ring, which has no fill to blend with) skips the blend.
    let dot = match fade {
        Some((from, t)) if !ring && t < 1.0 => mix_color(from, target, t),
        _ => target,
    };
    let border_color = if matches!(badge, Badge::Error(_)) {
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

    let mut card = div()
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
        // An inactive branch draws its whole card dimmed.
        .when(matches!(badge, Badge::Inactive), |card| card.opacity(0.4))
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
        });
    card = match detail {
        Detail::Title => {
            // One bold title, always 13 screen px so a distant card keeps its name.
            // Port dots stay at the layout offsets, so wires still end on ports.
            let block = layout::PORT_ROW * rows as f32;
            let port_top = (layout::PAD_Y + layout::HEADER + layout::SUMMARY) * zoom;
            let mut column = div()
                .absolute()
                .left(px(0.0))
                .right(px(0.0))
                .top(px(port_top))
                .h(px(block * zoom));
            if inputs.len() <= 1 {
                column = column.child(port_pair(
                    entity.clone(),
                    &id,
                    inputs.first(),
                    output.as_ref(),
                    0.0,
                    zoom,
                    detail,
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
                        detail,
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
                        detail,
                        graph,
                        None,
                        None,
                        &paint,
                    ));
                }
            }
            let (size, line) = layout::text_with_zoom(13.0 / zoom, 1.0);
            card.child(
                div()
                    .mt(px(layout::PAD_Y * zoom))
                    .h(px((layout::HEADER * zoom).max(size.as_f32())))
                    .px(px(10.0 * zoom))
                    .flex()
                    .items_center()
                    .truncate()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_size(size)
                    .line_height(line)
                    .text_color(paint.fg)
                    .child(title),
            )
            .child(column)
        }
        Detail::Compact | Detail::Full => {
            let (title_size, title_line) = layout::text_with_zoom(layout::TITLE_TEXT, zoom);
            card
                // The summary row keeps its height in every detail so the port
                // block and the wires stay where the layout puts them.
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
                                .truncate()
                                .text_size(title_size)
                                .line_height(title_line)
                                .text_color(paint.fg)
                                .child(title),
                        )
                        .child(badge_element(&id, badge, dot, paint.muted, zoom)),
                )
                .child(
                    div()
                        .h(px(layout::SUMMARY * zoom))
                        .px(px(10.0 * zoom))
                        .overflow_hidden()
                        .when(detail == Detail::Full, |row| {
                            let (size, line) = layout::text_with_zoom(layout::BODY_TEXT, zoom);
                            row.text_size(size)
                                .line_height(line)
                                .text_color(paint.muted)
                                .font_family(paint.mono.clone())
                                .child(summary)
                        }),
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
                            detail,
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
                                detail,
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
                                detail,
                                graph,
                                None,
                                None,
                                &paint,
                            ));
                        }
                    }
                    column
                })
                .when(detail == Detail::Full, |card| {
                    card.children(output_body(
                        node,
                        paint.frame.as_ref(),
                        selected,
                        reduced_motion,
                        &paint,
                        zoom,
                    ))
                })
        }
    };
    card.into_any_element()
}

fn output_body(
    node: &Node,
    frame: Option<&FrameView>,
    selected: bool,
    reduced_motion: bool,
    paint: &CardPaint,
    zoom: f32,
) -> Option<gpui_kit::AnyElement> {
    let (size, line) = layout::text_with_zoom(layout::BODY_TEXT, zoom);
    if matches!(
        node.feature.as_str(),
        "to_csv" | "to_excel" | "to_json" | "to_parquet" | "to_sql"
    ) {
        let (text, color) = match paint.note.as_deref() {
            Some(note) if note.starts_with("Wrote") => (note.to_string(), paint.success),
            Some(note) => (note.to_string(), paint.danger),
            None => ("Set a path, then run".to_string(), paint.muted),
        };
        return Some(
            div()
                .px(px(10.0 * zoom))
                .text_size(size)
                .line_height(line)
                .text_color(color)
                .child(text)
                .into_any_element(),
        );
    }
    if node.feature == "preview_table" {
        return Some(match frame {
            Some(frame) => mini_table(frame, paint, zoom),
            None => div()
                .px(px(10.0 * zoom))
                .text_size(size)
                .line_height(line)
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
                .px(px(10.0 * zoom))
                .text_size(size)
                .line_height(line)
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
                .h(px(168.0 * zoom))
                .px(px(6.0 * zoom))
                .pb(px(6.0 * zoom))
                .child(feature_chart(
                    &mark,
                    points,
                    colors,
                    paint.fg,
                    reduced_motion,
                    selected,
                    &node.id,
                    "",
                ))
                .into_any_element(),
            Err(message) => div()
                .px(px(10.0 * zoom))
                .text_size(size)
                .line_height(line)
                .text_color(paint.danger)
                .child(message)
                .into_any_element(),
        },
    )
}

fn mini_table(frame: &FrameView, paint: &CardPaint, zoom: f32) -> gpui_kit::AnyElement {
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
        zoom,
    ));
    for row in frame.rows.iter().take(4) {
        let cells = (0..count)
            .map(|index| row.get(index).map(graph::cell_text).unwrap_or_default())
            .collect::<Vec<_>>();
        grid = grid.child(mini_row(&cells, false, paint, zoom));
    }
    grid.into_any_element()
}

fn mini_row(cells: &[String], header: bool, paint: &CardPaint, zoom: f32) -> gpui_kit::Div {
    let (size, line) = layout::text_with_zoom(layout::TYPE_TEXT, zoom);
    let mut row = div().flex().flex_row().gap(px(6.0 * zoom));
    for cell in cells {
        let numeric = !header && cell.parse::<f64>().is_ok();
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(size)
                .line_height(line)
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
    detail: layout::Detail,
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
            detail,
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
            detail,
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
    detail: layout::Detail,
    graph: &graph::Graph,
    link_from: Option<(String, String)>,
    snap_target: Option<(String, String)>,
    paint: &CardPaint,
) -> gpui_kit::AnyElement {
    let name = port.name.clone();
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
    // Port labels and type labels only when the camera is near enough
    // for them to be read.
    if detail != layout::Detail::Full {
        let node_id = node_id.to_string();
        return div()
            .id(SharedString::from(format!(
                "{}-{node_id}-{name}",
                if side == Side::In { "in" } else { "out" }
            )))
            .flex()
            .items_center()
            .h(px(layout::PORT_ROW * zoom))
            .child(dot)
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                cx.stop_propagation();
                entity.update(cx, |this, cx| {
                    this.port_down(&node_id, &name, side, event, window, cx);
                });
            })
            .into_any_element();
    }
    let generic = name == "in" || name == "out" || name.is_empty();
    let (label_size, label_line) = layout::text_with_zoom(layout::BODY_TEXT, zoom);
    let (type_size, type_line) = layout::text_with_zoom(layout::TYPE_TEXT, zoom);
    let label = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0 * zoom))
        .max_w(px(78.0 * zoom))
        .min_w(px(0.0))
        .child(
            div()
                .truncate()
                .text_size(label_size)
                .line_height(label_line)
                .text_color(if dim {
                    paint.muted.opacity(0.4)
                } else {
                    paint.muted
                })
                .when(!generic, |label| label.child(name.clone())),
        )
        .child(
            div()
                .truncate()
                .text_size(type_size)
                .line_height(type_line)
                .font_family(paint.mono.clone())
                .text_color(if dim {
                    paint.muted.opacity(0.4)
                } else {
                    paint.muted.opacity(0.75)
                })
                .child(shorten_types(&port_caption("", &port.accepts))),
        );
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
    let (text_size, text_line) = layout::text_with_zoom(layout::BODY_TEXT, zoom);
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
        .text_size(text_size)
        .line_height(text_line)
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
