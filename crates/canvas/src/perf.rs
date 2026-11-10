//! Viewport culling and the synthetic graph for the performance bench.
//!
//! Pure functions over the document model, so they run without GPUI. The
//! render path calls the same helpers the bench times.

use std::collections::HashMap;

use crate::layout::{Cubic, PortSlot, Side, port_slots, wire_cubic};
use graph::{Camera, Edge, Graph, Node, Point, Rect};

/// Window pixels added around the viewport before the intersect test.
pub const MARGIN_PX: f32 = 64.0;

/// Grow `rect` by `margin` on every side.
pub fn grow(rect: Rect, margin: f32) -> Rect {
    Rect {
        x: rect.x - margin,
        y: rect.y - margin,
        w: rect.w + margin * 2.0,
        h: rect.h + margin * 2.0,
    }
}

/// True when the two rectangles share at least one point.
pub fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Card box of `node`, projected to the window by `camera`.
pub fn node_window_rect(node: &Node, camera: Camera) -> Rect {
    let (origin, w, h) = crate::layout::card_rect(node);
    let at = camera.to_window(origin);
    Rect {
        x: at.x,
        y: at.y,
        w: w * camera.zoom,
        h: h * camera.zoom,
    }
}

/// Bounding box of every control point of the cubic chain, in canvas space.
pub fn chain_control_box(chain: &[Cubic]) -> Rect {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for cubic in chain {
        for point in [cubic.from, cubic.c1, cubic.c2, cubic.to] {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
    }
    if min_x > max_x {
        return Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
    }
    Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    }
}

/// `chain_control_box` projected through `camera`, still in window space.
pub fn chain_window_box(chain: &[Cubic], camera: Camera) -> Rect {
    let box_ = chain_control_box(chain);
    let at = camera.to_window(Point::new(box_.x, box_.y));
    Rect {
        x: at.x,
        y: at.y,
        w: box_.w * camera.zoom,
        h: box_.h * camera.zoom,
    }
}

/// Indices of `rects` (already window space) that intersect the viewport with
/// `margin` added. An unset viewport returns every index.
pub fn visible_rects(rects: &[Rect], viewport: Option<Rect>, margin: f32) -> Vec<usize> {
    let Some(view) = viewport else {
        return (0..rects.len()).collect();
    };
    let view = grow(view, margin);
    (0..rects.len())
        .filter(|index| {
            let rect = rects[*index];
            rect.w.max(rect.h) >= 0.0 && rects_intersect(rect, view)
        })
        .collect()
}

/// Indices into `graph.nodes` whose card rect (plus `margin`) meets the
/// viewport in window space.
pub fn visible_nodes(graph: &Graph, camera: &Camera, viewport: Rect, margin: f32) -> Vec<usize> {
    let rects: Vec<Rect> = graph
        .nodes
        .iter()
        .map(|node| node_window_rect(node, *camera))
        .collect();
    visible_rects(&rects, Some(viewport), margin)
}

/// Control-point box of `edge`: the port centers, every reroute, and the
/// tangent handles of each segment. `None` when either endpoint is missing.
pub fn edge_control_box(edge: &Edge, slots: &HashMap<String, Vec<PortSlot>>) -> Option<Rect> {
    let find = |port: &str, side: Side| -> Option<Point> { slot_center(edge, side, port, slots) };
    let from = find(&edge.from_port, Side::Out);
    let to = find(&edge.to_port, Side::In);
    let (from, to) = (from?, to?);
    let mut min_x = from.x;
    let mut min_y = from.y;
    let mut max_x = from.x;
    let mut max_y = from.y;
    let mut fold = |point: Point| {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    };
    let mut start = from;
    for reroute in &edge.reroutes {
        let point = Point::new(reroute[0], reroute[1]);
        fold_box(wire_cubic(start, point), &mut fold);
        fold(point);
        start = point;
    }
    fold_box(wire_cubic(start, to), &mut fold);
    fold(to);
    Some(Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    })
}

fn fold_box(cubic: Cubic, fold: &mut impl FnMut(Point)) {
    fold(cubic.c1);
    fold(cubic.c2);
}

fn slot_center(
    edge: &Edge,
    side: Side,
    port: &str,
    slots: &HashMap<String, Vec<PortSlot>>,
) -> Option<Point> {
    let key = match side {
        Side::Out => edge.from.as_str(),
        Side::In => edge.to.as_str(),
    };
    let node_slots = slots.get(key)?;
    node_slots
        .iter()
        .find(|slot| slot.side == side && slot.name == port)
        .map(|slot| slot.center)
}

/// Port slots of every node, indexed by node id. Callers that probe many
/// edges rebuild this once instead of walking the node list per edge.
pub fn slots_of(graph: &Graph) -> HashMap<String, Vec<PortSlot>> {
    graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), port_slots(node)))
        .collect()
}

/// A canvas-space rectangle projected through `camera`.
pub fn projected(rect: Rect, camera: Camera) -> Rect {
    let at = camera.to_window(Point::new(rect.x, rect.y));
    Rect {
        x: at.x,
        y: at.y,
        w: rect.w * camera.zoom,
        h: rect.h * camera.zoom,
    }
}

/// Indices into `graph.edges` whose control-point box (plus `margin`) meets
/// the viewport in window space.
pub fn visible_edges(graph: &Graph, camera: &Camera, viewport: Rect, margin: f32) -> Vec<usize> {
    let slots = slots_of(graph);
    let view = grow(viewport, margin);
    graph
        .edges
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| {
            let at = edge_control_box(edge, &slots)?;
            let rect = projected(at, *camera);
            rects_intersect(rect, view).then_some(index)
        })
        .collect()
}

/// A layered DAG of catalog features. Roughly `nodes / 10` read sources on
/// layer zero, then single-in single-out steps with a merge every fourth
/// node feeding on an extra thread. Grid positions with 260 x 150 spacing,
/// about 1.2 edges per node, all deterministic on `seed`.
pub fn synthetic_graph(nodes: usize, seed: u64) -> Graph {
    let total = nodes.max(3);
    let mut random = Random { state: seed | 1 };
    let columns = (total / 10).clamp(2, 24);
    let mut graph = Graph::default();
    let mut ids: Vec<Vec<String>> = Vec::new();
    let singles = ["head", "query", "dropna", "change_headers", "sort_values"];

    for row in 0.. {
        let mut ids_row = Vec::with_capacity(columns);
        for column in 0..columns {
            if graph.nodes.len() >= total {
                break;
            }
            let feature = match row {
                0 => "read_csv",
                _ => {
                    let index = graph.nodes.len();
                    if index % 4 == 3 {
                        "merge"
                    } else {
                        singles[index % singles.len()]
                    }
                }
            };
            let id = format!("n{}", graph.nodes.len() + 1);
            let position = Point::new(48.0 + column as f32 * 260.0, 48.0 + row as f32 * 150.0);
            graph.nodes.push(Node::new(id.clone(), feature, position));
            // Layer zero has no wire into it; deeper rows always have one
            // from the cell above, merges add a second one.
            if row > 0 {
                let port = if feature == "merge" { "left" } else { "in" };
                graph
                    .edges
                    .push(Edge::between(&ids[row - 1][column], "out", &id, port));
            }
            ids_row.push(id.clone());
            if feature == "merge" {
                let pair = (column + 1 + random.next_usize(columns - 1)) % columns;
                if pair != column {
                    graph
                        .edges
                        .push(Edge::between(&ids[row - 1][pair], "out", &id, "right"));
                }
            }
        }
        if ids_row.is_empty() {
            break;
        }
        ids.push(ids_row);
    }

    graph
}

struct Random {
    state: u64,
}

impl Random {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_usize(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}
