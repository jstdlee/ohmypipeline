//! Group frames: geometry, collapse routing, and the header element.
//!
//! A frame is a visual group only. Membership is derived from positions, so
//! the file stores one rectangle and a title. The frame paints as one element
//! layer: a rounded body with a 28 px header strip, both placed behind the
//! cards, so the translucent tint still shows the wires running through it.

use std::collections::BTreeMap;

use crate::layout::Cubic;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::{
    Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use graph::{Edge, Frame, Graph, Node, Point, frame_members};

/// Height of the header strip, in canvas pixels.
pub const HEADER_H: f32 = 28.0;
/// Drag-out padding used by Group.
pub const GROUP_PAD: f32 = 24.0;
const BODY_OPACITY: f32 = 0.06;
const BORDER_OPACITY: f32 = 0.35;
const HEADER_OPACITY: f32 = 0.10;
const RADIUS: f32 = 8.0;
/// Half width of the resize hit strips, in window pixels.
const HANDLE: f32 = 8.0;

/// Theme color behind a swatch name. `c1`..`c6` reuse the mark palette;
/// unknown names fall back to the muted foreground.
pub fn swatch_color(
    swatch: &str,
    chart: &[gpui_kit::Hsla; 6],
    muted: gpui_kit::Hsla,
) -> gpui_kit::Hsla {
    match swatch {
        "c1" => chart[0],
        "c2" => chart[1],
        "c3" => chart[2],
        "c4" => chart[3],
        "c5" => chart[4],
        "c6" => chart[5],
        _ => muted,
    }
}

/// Every swatch name a frame may carry, in menu order.
pub const SWATCHES: [&str; 7] = ["neutral", "c1", "c2", "c3", "c4", "c5", "c6"];

/// Label of a swatch in the Color menu.
pub fn swatch_label(swatch: &str) -> &'static str {
    match swatch {
        "neutral" => "Neutral",
        "c1" => "Swatch 1",
        "c2" => "Swatch 2",
        "c3" => "Swatch 3",
        "c4" => "Swatch 4",
        "c5" => "Swatch 5",
        "c6" => "Swatch 6",
        _ => "Neutral",
    }
}

/// Nodes hidden by collapsed frames, mapped to the frame id that hides them.
pub fn collapsed_members(graph: &Graph) -> BTreeMap<String, String> {
    let mut hidden = BTreeMap::new();
    for frame in graph.frames.iter().filter(|frame| frame.collapsed) {
        for member in frame_members(graph, &frame.id) {
            hidden.insert(member, frame.id.clone());
        }
    }
    hidden
}

/// Point on the header's left or right edge nearest `toward`. A wire from a
/// hidden member to a live node leaves the header here.
pub fn header_edge_point(frame: &Frame, toward: Point) -> Point {
    let mid_y = frame.position[1] + HEADER_H * 0.5;
    let center = frame.position[0] + frame.size[0] * 0.5;
    if toward.x < center {
        Point::new(frame.position[0], mid_y)
    } else {
        Point::new(frame.position[0] + frame.size[0], mid_y)
    }
}

/// Wire chain for `edge`, replacing endpoints hidden by collapsed frames
/// with the header edge midpoint. A wire with both ends hidden draws nothing.
pub fn edge_wire_path_routed(
    nodes: &[Node],
    frames: &[Frame],
    hidden: &BTreeMap<String, String>,
    edge: &Edge,
) -> Option<Vec<Cubic>> {
    let source = nodes.iter().find(|node| node.id == edge.from)?;
    let target = nodes.iter().find(|node| node.id == edge.to)?;
    let source_slot = crate::layout::port_slots(source)
        .into_iter()
        .find(|slot| slot.name == edge.from_port && slot.side == crate::layout::Side::Out)?
        .center;
    let target_slot = crate::layout::port_slots(target)
        .into_iter()
        .find(|slot| slot.name == edge.to_port && slot.side == crate::layout::Side::In)?
        .center;
    if hidden.contains_key(&edge.from) && hidden.contains_key(&edge.to) {
        return None;
    }
    let frame_of = |id: &str| -> Option<&Frame> {
        hidden
            .get(id)
            .and_then(|frame_id| frames.iter().find(|frame| &frame.id == frame_id))
    };
    let from = match frame_of(&edge.from) {
        Some(frame) => header_edge_point(frame, target_slot),
        None => source_slot,
    };
    let to = match frame_of(&edge.to) {
        Some(frame) => header_edge_point(frame, source_slot),
        None => target_slot,
    };
    let reroutes: Vec<Point> = edge
        .reroutes
        .iter()
        .map(|point| Point::new(point[0], point[1]))
        .collect();
    Some(crate::layout::wire_path(from, &reroutes, to))
}

/// Endpoint centers an edge connects in canvas space, substituting the header
/// edge point for a hidden card. `None` when both ends live in collapsed
/// frames or a port is missing.
pub fn edge_endpoints(
    nodes: &[Node],
    frames: &[Frame],
    hidden: &BTreeMap<String, String>,
    edge: &Edge,
) -> Option<(Point, Point)> {
    if hidden.contains_key(&edge.from) && hidden.contains_key(&edge.to) {
        return None;
    }
    let source = nodes.iter().find(|node| node.id == edge.from)?;
    let target = nodes.iter().find(|node| node.id == edge.to)?;
    let source_slot = crate::layout::port_slots(source)
        .into_iter()
        .find(|slot| slot.name == edge.from_port && slot.side == crate::layout::Side::Out)?
        .center;
    let target_slot = crate::layout::port_slots(target)
        .into_iter()
        .find(|slot| slot.name == edge.to_port && slot.side == crate::layout::Side::In)?
        .center;
    let frame_of = |id: &str| -> Option<&Frame> {
        hidden
            .get(id)
            .and_then(|frame_id| frames.iter().find(|frame| &frame.id == frame_id))
    };
    let from = match frame_of(&edge.from) {
        Some(frame) => header_edge_point(frame, target_slot),
        None => source_slot,
    };
    let to = match frame_of(&edge.to) {
        Some(frame) => header_edge_point(frame, source_slot),
        None => target_slot,
    };
    Some((from, to))
}

/// Which part of a frame the drag-out started on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    Corner,
    Right,
    Bottom,
}

/// Data the frame element needs, computed once per render.
#[derive(Clone)]
pub struct FramePaint {
    pub frame: Frame,
    /// Animated visible height, in canvas pixels.
    pub height: f32,
    pub selected: bool,
    pub members: usize,
    pub color: gpui_kit::Hsla,
}

pub struct FrameTheme {
    pub fg: gpui_kit::Hsla,
    pub muted: gpui_kit::Hsla,
    pub accent: gpui_kit::Hsla,
    pub card: gpui_kit::Hsla,
}

/// The whole frame in one element layer: a rounded, bordered body with the
/// translucent tint, topped by the 28 canvas pixel header strip with the
/// title, chevron, and resize handles. The element sits behind the cards in
/// paint order, so the body never covers its members.
pub fn frame_element(
    entity: Entity<crate::model::CanvasModel>,
    paint: FramePaint,
    theme: FrameTheme,
    camera: graph::Camera,
    collapsed_now: bool,
) -> gpui_kit::AnyElement {
    let zoom = camera.zoom;
    let at = camera.to_window(Point::new(paint.frame.position[0], paint.frame.position[1]));
    let width = px(paint.frame.size[0] * zoom);
    let height = px(paint.height * zoom);
    let header_h = px(HEADER_H * zoom);
    let radius = px(RADIUS * zoom);
    let id = paint.frame.id.clone();
    let (title_size, title_line) = crate::layout::text_with_zoom(crate::layout::TITLE_TEXT, zoom);
    let chevron = if paint.frame.collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    let border_color = if paint.selected {
        theme.accent.opacity(0.9)
    } else {
        paint.color.opacity(BORDER_OPACITY)
    };

    let mut header = div()
        .id(SharedString::from(format!("frame-header-{id}")))
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .right(px(0.0))
        .h(header_h)
        .flex()
        .flex_row()
        .items_center()
        .px(px(10.0 * zoom))
        .gap(px(6.0 * zoom))
        .bg(paint.color.opacity(HEADER_OPACITY))
        .border_b_1()
        .border_color(paint.color.opacity(BORDER_OPACITY))
        .rounded_tl(radius)
        .rounded_tr(radius)
        .cursor(gpui_kit::CursorStyle::Arrow)
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            let id = id.clone();
            move |event, window, cx| {
                cx.stop_propagation();
                entity.update(cx, |this, cx| {
                    if event.click_count >= 2 {
                        this.start_frame_rename(&id, window, cx);
                    } else {
                        this.frame_header_down(&id, event, window, cx);
                    }
                });
            }
        });
    header = header.child(
        div()
            .size(px(9.0 * zoom))
            .flex_shrink_0()
            .rounded(px(2.0))
            .bg(paint.color),
    );
    header = header.child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .truncate()
            .font_weight(gpui_kit::FontWeight::MEDIUM)
            .text_size(title_size)
            .line_height(title_line)
            .text_color(theme.fg)
            .child(paint.frame.title.clone()),
    );
    if collapsed_now {
        header = header.child(
            div()
                .flex_shrink_0()
                .text_size(title_size)
                .line_height(title_line)
                .text_color(theme.muted)
                .child(count_label(paint.members)),
        );
    }
    header = header.child(
        div()
            .id(SharedString::from(format!("frame-chevron-{id}")))
            .flex_shrink_0()
            .size(px(18.0 * zoom.max(0.8)))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .text_color(theme.muted)
            .hover(|style| style.bg(theme.card))
            .child(
                gpui_kit::component::Icon::new(chevron)
                    .with_size(px(12.0 * zoom.max(0.8)))
                    .into_element(),
            )
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                let id = id.clone();
                move |_event, _window, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        this.toggle_frame_collapsed(&id, cx);
                    });
                }
            }),
    );
    let header = header.context_menu({
        let entity = entity.clone();
        let id = id.clone();
        move |menu, window, cx| frame_menu(menu, id.clone(), entity.clone(), window, cx)
    });

    let mut root = div()
        .absolute()
        .left(px(at.x))
        .top(px(at.y))
        .w(width)
        .h(height)
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .rounded(radius)
                .bg(paint.color.opacity(BODY_OPACITY))
                .border_1()
                .border_color(border_color),
        )
        .child(header);
    if paint.selected {
        let right_strip = div()
            .id(SharedString::from(format!("frame-resize-r-{id}")))
            .absolute()
            .right(px(-HANDLE * 0.5))
            .top(header_h)
            .bottom(px(0.0))
            .w(px(HANDLE))
            .cursor(gpui_kit::CursorStyle::ResizeRight)
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                let id = id.clone();
                move |event, _window, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        this.frame_resize_down(&id, ResizeEdge::Right, event, cx);
                    });
                }
            });
        let bottom_strip = div()
            .id(SharedString::from(format!("frame-resize-b-{id}")))
            .absolute()
            .bottom(px(-HANDLE * 0.5))
            .left(px(0.0))
            .right(px(0.0))
            .h(px(HANDLE))
            .cursor(gpui_kit::CursorStyle::ResizeDown)
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                let id = id.clone();
                move |event, _window, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        this.frame_resize_down(&id, ResizeEdge::Bottom, event, cx);
                    });
                }
            });
        let corner = div()
            .id(SharedString::from(format!("frame-resize-c-{id}")))
            .absolute()
            .bottom(px(-HANDLE * 0.5))
            .right(px(-HANDLE * 0.5))
            .size(px(HANDLE * 2.0))
            .rounded(px(3.0))
            .bg(theme.accent)
            .cursor(gpui_kit::CursorStyle::ResizeUpLeftDownRight)
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                let id = id.clone();
                move |event, _window, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        this.frame_resize_down(&id, ResizeEdge::Corner, event, cx);
                    });
                }
            });
        root = root.child(right_strip).child(bottom_strip).child(corner);
    }
    root.into_any_element()
}

fn count_label(members: usize) -> String {
    if members == 1 {
        "1 node".to_string()
    } else {
        format!("{members} nodes")
    }
}

/// Context menu of the header: rename, color, collapse, ungroup, select.
fn frame_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    frame_id: String,
    entity: Entity<crate::model::CanvasModel>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<gpui_kit::component::menu::PopupMenu>,
) -> gpui_kit::component::menu::PopupMenu {
    use gpui_kit::component::menu::PopupMenuItem;
    let collapsed = entity
        .read(cx)
        .editor()
        .graph()
        .frames
        .iter()
        .any(|frame| frame.id == frame_id && frame.collapsed);
    let rename = entity.clone();
    let toggle = entity.clone();
    let ungroup = entity.clone();
    let select = entity.clone();
    let rename_id = frame_id.clone();
    let menu = menu
        .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
            rename.update(cx, |this, cx| {
                this.start_frame_rename(&rename_id, window, cx)
            });
        }))
        .separator();
    let swatches = entity.clone();
    let color_menu_id = frame_id.clone();
    let mut menu = menu.submenu("Color", window, cx, move |menu, _, _| {
        let mut menu = menu;
        for swatch in SWATCHES {
            let swatch = swatch.to_string();
            menu = menu.item(PopupMenuItem::new(swatch_label(&swatch)).on_click({
                let entity = swatches.clone();
                let frame = color_menu_id.clone();
                let name = swatch.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.editor_mut().set_frame_swatch(&frame, name.clone());
                        this.notify_edited(cx);
                    });
                }
            }));
        }
        menu
    });
    let toggle_id = frame_id.clone();
    let ungroup_id = frame_id.clone();
    let select_id = frame_id.clone();
    menu = menu
        .item(
            PopupMenuItem::new(if collapsed { "Expand" } else { "Collapse" }).on_click(
                move |_, _, cx| {
                    toggle.update(cx, |this, cx| {
                        this.toggle_frame_collapsed(&toggle_id, cx);
                    });
                },
            ),
        )
        .item(PopupMenuItem::new("Ungroup").on_click(move |_, _, cx| {
            ungroup.update(cx, |this, cx| {
                this.editor_mut()
                    .delete_frames(std::slice::from_ref(&ungroup_id));
                this.selection.retain(|id| id != &ungroup_id);
                this.notify_edited(cx);
            });
        }))
        .item(
            PopupMenuItem::new("Select members").on_click(move |_, _, cx| {
                select.update(cx, |this, cx| this.select_frame_members(&select_id, cx));
            }),
        );
    menu
}
