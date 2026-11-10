//! The minimap: a small map of the graph with the viewport rectangle.
//!
//! Click or drag inside it moves the camera. The mapping fits the union of
//! the graph bounds and the viewport with an 8 percent margin, so the map
//! never clips either.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Styled as _,
    div, px,
};
use graph::{Camera, Graph, Point, Rect};

/// Size of the map panel, in window pixels.
pub const SIZE: [f32; 2] = [200.0, 132.0];
/// Distance from the panel to the canvas panel edges.
pub const MARGIN: f32 = 12.0;
/// Empty margin kept around the mapped content.
const FIT_MARGIN: f32 = 0.08;
const RADIUS: f32 = 8.0;

/// Mapping between canvas points and minimap points.
#[derive(Clone, Copy, Debug)]
pub struct Map {
    /// Content bounds in canvas pixels, already inflated by the margin.
    pub bounds: Rect,
    /// Canvas pixels per minimap pixel.
    pub scale: f32,
    /// Top-left of the mapped bounds inside the panel, in panel pixels.
    pub offset: Point,
}

impl Map {
    /// Fit `content` (the union of graph and viewport bounds) into `size`.
    pub fn fit(content: Rect, size: [f32; 2]) -> Self {
        let padded = Rect {
            x: content.x - content.w * FIT_MARGIN,
            y: content.y - content.h * FIT_MARGIN,
            w: content.w * (1.0 + FIT_MARGIN * 2.0),
            h: content.h * (1.0 + FIT_MARGIN * 2.0),
        };
        let scale = if padded.w <= 0.0 || padded.h <= 0.0 {
            1.0
        } else {
            (size[0] / padded.w).min(size[1] / padded.h)
        };
        let offset = Point::new(
            (size[0] - padded.w * scale) * 0.5,
            (size[1] - padded.h * scale) * 0.5,
        );
        Self {
            bounds: padded,
            scale,
            offset,
        }
    }

    pub fn to_panel(self, canvas: Point) -> Point {
        Point::new(
            (canvas.x - self.bounds.x) * self.scale + self.offset.x,
            (canvas.y - self.bounds.y) * self.scale + self.offset.y,
        )
    }

    pub fn to_canvas(self, panel: Point) -> Point {
        Point::new(
            (panel.x - self.offset.x) / self.scale + self.bounds.x,
            (panel.y - self.offset.y) / self.scale + self.bounds.y,
        )
    }
}

/// The canvas-space viewport rectangle of a camera in a view of `view` px.
pub fn viewport_rect(camera: Camera, view: (f32, f32)) -> Rect {
    let top_left = camera.to_canvas(Point::new(0.0, 0.0));
    let bottom_right = camera.to_canvas(Point::new(view.0, view.1));
    Rect {
        x: top_left.x.min(bottom_right.x),
        y: top_left.y.min(bottom_right.y),
        w: (bottom_right.x - top_left.x).abs(),
        h: (bottom_right.y - top_left.y).abs(),
    }
}

/// Union of the graph bounds and the viewport, the content the map shows.
pub fn content_bounds(
    graph: &Graph,
    card_height: impl Fn(&graph::Node) -> f32,
    view: (f32, f32),
) -> Rect {
    let viewport = viewport_rect(graph.camera, view);
    match graph::bounds_of(graph, &[], card_height) {
        Some(nodes) => nodes.union(viewport),
        None => viewport,
    }
}

/// The overlay element. `alpha` drives the fade, `map` the geometry, and
/// `view` is the canvas panel size in window pixels.
pub fn minimap_element(
    entity: Entity<crate::model::CanvasModel>,
    graph: &Graph,
    selection: &[String],
    map: Map,
    alpha: f32,
    view: (f32, f32),
    cx: &gpui_kit::App,
) -> gpui_kit::AnyElement {
    let theme = cx.theme();
    let background = theme.background.opacity(0.92);
    let border = theme.border;
    let muted = theme.muted_foreground;
    let accent = theme.accent;
    let chart = [
        theme.chart_1,
        theme.chart_2,
        theme.chart_3,
        theme.chart_4,
        theme.chart_5,
        theme.chart_5,
    ];
    let hidden = crate::frames::collapsed_members(graph);

    let mut panel = div()
        .id("minimap")
        .absolute()
        .bottom(px(MARGIN))
        .right(px(MARGIN))
        .w(px(SIZE[0]))
        .h(px(SIZE[1]))
        .rounded(px(RADIUS))
        .bg(background)
        .border_1()
        .border_color(border)
        .shadow_sm()
        .overflow_hidden()
        .opacity(alpha)
        .cursor(gpui_kit::CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            move |event, _window, cx| {
                cx.stop_propagation();
                entity.update(cx, |this, cx| this.minimap_down(event, cx));
            }
        })
        .on_mouse_move({
            let entity = entity.clone();
            move |event: &gpui_kit::MouseMoveEvent, _window, cx| {
                if !event.dragging() {
                    return;
                }
                entity.update(cx, |this, cx| this.minimap_drag_move(event, cx));
            }
        });

    for frame in &graph.frames {
        let color = crate::frames::swatch_color(&frame.swatch, &chart, muted);
        let at = map.to_panel(Point::new(frame.position[0], frame.position[1]));
        let w = (frame.size[0] * map.scale).max(2.0);
        let h = (frame.size[1] * map.scale).max(2.0);
        panel = panel.child(
            div()
                .absolute()
                .left(px(at.x))
                .top(px(at.y))
                .w(px(w))
                .h(px(h.min(SIZE[1])))
                .rounded(px(2.0))
                .bg(color.opacity(0.25)),
        );
    }
    for node in &graph.nodes {
        if hidden.contains_key(&node.id) {
            continue;
        }
        let selected = selection.iter().any(|id| id == &node.id);
        let at = map.to_panel(node.position_point());
        let w = (graph::card_width(&node.feature) * map.scale).max(2.0);
        let h = (crate::card_height(node) * map.scale).max(2.0);
        panel = panel.child(
            div()
                .absolute()
                .left(px(at.x))
                .top(px(at.y))
                .w(px(w))
                .h(px(h))
                .rounded(px(1.0))
                .bg(if selected {
                    accent
                } else {
                    muted.opacity(0.45)
                }),
        );
    }
    let viewport = viewport_rect(graph.camera, view);
    let top_left = map.to_panel(Point::new(viewport.x, viewport.y));
    let bottom = map.to_panel(Point::new(viewport.x + viewport.w, viewport.y + viewport.h));
    let w = (bottom.x - top_left.x).max(4.0);
    let h = (bottom.y - top_left.y).max(4.0);
    panel = panel.child(
        div()
            .absolute()
            .left(px(top_left.x))
            .top(px(top_left.y))
            .w(px(w))
            .h(px(h))
            .border_1()
            .border_color(accent)
            .rounded(px(2.0))
            .bg(accent.opacity(0.08)),
    );
    panel.into_any_element()
}
