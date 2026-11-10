//! Card and wire geometry in canvas pixels.
//!
//! The file stores node positions, not curves. A wire's ends are the port
//! slots of the cards it joins, so moving a card moves the wire.

use graph::{Camera, Edge, Node, Point, card_width};

pub const PORT_ROW: f32 = 22.0;
pub const HEADER: f32 = 28.0;
pub const SUMMARY: f32 = 16.0;
pub const PAD_Y: f32 = 8.0;

/// Text bases. A card draws every glyph at `BASE * zoom` so the letters
/// shrink and grow with the same camera as the card box.
pub const TITLE_TEXT: f32 = 12.0;
pub const BODY_TEXT: f32 = 11.0;
pub const TYPE_TEXT: f32 = 10.0;

/// How much of a card the camera can still ask a card to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Everything: title, summary, port labels, inline body.
    Full,
    /// Title and port dots only, so distant cards stay quiet.
    Compact,
    /// A single bold title. For very distant cards.
    Title,
}

pub fn detail_for(zoom: f32) -> Detail {
    if zoom >= 0.55 {
        Detail::Full
    } else if zoom >= 0.3 {
        Detail::Compact
    } else {
        Detail::Title
    }
}

/// Line height that tracks a text base the way the card box tracks the zoom.
pub fn text_with_zoom(base: f32, zoom: f32) -> (gpui_kit::Pixels, gpui_kit::Pixels) {
    (gpui_kit::px(base * zoom), gpui_kit::px(base * zoom * 1.25))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    In,
    Out,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortSlot {
    pub name: String,
    pub side: Side,
    /// Canvas-pixel center of the port, absolute.
    pub center: Point,
}

pub fn card_height(node: &Node) -> f32 {
    let rows = node
        .input_ports()
        .len()
        .max(usize::from(node.output_port().is_some()))
        .max(1) as f32;
    let extra = match node.feature.as_str() {
        "preview_table" => 108.0,
        "chart" => 196.0,
        "to_csv" | "to_excel" | "to_json" | "to_parquet" | "to_sql" => 28.0,
        _ => 0.0,
    };
    HEADER + SUMMARY + PAD_Y + PORT_ROW * rows + PAD_Y + extra
}

pub fn card_rect(node: &Node) -> (Point, f32, f32) {
    (
        node.position_point(),
        card_width(&node.feature),
        card_height(node),
    )
}

pub fn port_slots(node: &Node) -> Vec<PortSlot> {
    let origin = node.position_point();
    let inputs = node.input_ports();
    let output = node.output_port();
    let rows = inputs.len().max(usize::from(output.is_some())).max(1);
    let top = origin.y + HEADER + SUMMARY + PAD_Y;
    let mut slots = Vec::new();
    for (index, port) in inputs.iter().enumerate() {
        slots.push(PortSlot {
            name: port.name.clone(),
            side: Side::In,
            center: Point::new(origin.x, top + PORT_ROW * index as f32 + PORT_ROW * 0.5),
        });
    }
    if let Some(port) = output {
        let y = top + (rows as f32 * PORT_ROW) * 0.5;
        slots.push(PortSlot {
            name: port.name,
            side: Side::Out,
            center: Point::new(origin.x + card_width(&node.feature), y),
        });
    }
    slots
}

pub fn slot<'a>(slots: &'a [PortSlot], name: &str, side: Side) -> Option<&'a PortSlot> {
    slots
        .iter()
        .find(|slot| slot.name == name && slot.side == side)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cubic {
    pub from: Point,
    pub c1: Point,
    pub c2: Point,
    pub to: Point,
}

/// Leaves each port along its edge. The file does not store this curve.
///
/// A forward wire uses a short horizontal tangent. A wire whose target sits
/// behind the source bows through the gap instead of swinging back across
/// the source card.
pub fn wire_cubic(from: Point, to: Point) -> Cubic {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx >= 32.0 {
        let pull = (dx * 0.45).clamp(28.0, 120.0);
        Cubic {
            from,
            c1: Point::new(from.x + pull, from.y),
            c2: Point::new(to.x - pull, to.y),
            to,
        }
    } else {
        let bow = dy.abs().clamp(48.0, 140.0);
        let side = if dy >= 0.0 { 1.0 } else { -1.0 };
        Cubic {
            from,
            c1: Point::new(from.x + 36.0, from.y + bow * 0.65 * side),
            c2: Point::new(to.x - 36.0, to.y - bow * 0.35 * side),
            to,
        }
    }
}

pub fn cubic_point(cubic: Cubic, t: f32) -> Point {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    Point::new(
        a * cubic.from.x + b * cubic.c1.x + c * cubic.c2.x + d * cubic.to.x,
        a * cubic.from.y + b * cubic.c1.y + c * cubic.c2.y + d * cubic.to.y,
    )
}

/// The full wire as a chain of cubics through each reroute point.
/// Segment `i` joins `points[i]` and `points[i + 1]`, where `points` is
/// `from`, the reroutes in order, then `to`. Each segment follows the same
/// tangent rule as [`wire_cubic`].
pub fn wire_path(from: Point, reroutes: &[Point], to: Point) -> Vec<Cubic> {
    let mut points = Vec::with_capacity(reroutes.len() + 2);
    points.push(from);
    points.extend_from_slice(reroutes);
    points.push(to);
    let mut path = Vec::with_capacity(points.len().saturating_sub(1));
    for pair in points.windows(2) {
        path.push(wire_cubic(pair[0], pair[1]));
    }
    path
}

/// Nearest segment of the chain within `radius`, as the segment index.
/// `None` when the point misses every segment.
pub fn wire_hit(path: &[Cubic], point: Point, radius: f32) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (index, cubic) in path.iter().enumerate() {
        for step in 0..=24 {
            let at = cubic_point(*cubic, step as f32 / 24.0);
            let dx = at.x - point.x;
            let dy = at.y - point.y;
            let distance = dx * dx + dy * dy;
            if distance <= radius * radius && best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, index));
            }
        }
    }
    best.map(|(_, index)| index)
}

/// The wire of `edge` as a chain, including its stored reroute points.
/// The first segment of the chain is the plain `edge_cubic` of before.
pub fn edge_wire_path(nodes: &[Node], edge: &Edge) -> Option<Vec<Cubic>> {
    let source = nodes.iter().find(|node| node.id == edge.from)?;
    let target = nodes.iter().find(|node| node.id == edge.to)?;
    let source_slots = port_slots(source);
    let target_slots = port_slots(target);
    let from = slot(&source_slots, &edge.from_port, Side::Out)?;
    let to = slot(&target_slots, &edge.to_port, Side::In)?;
    let reroutes: Vec<Point> = edge
        .reroutes
        .iter()
        .map(|point| Point::new(point[0], point[1]))
        .collect();
    Some(wire_path(from.center, &reroutes, to.center))
}

pub fn to_local(camera: Camera, canvas: Point) -> Point {
    camera.to_window(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph::Node;

    #[test]
    fn moving_a_card_moves_the_wire_without_a_stored_curve() {
        let mut source = Node::new("n1", "read_csv", Point::new(16.0, 32.0));
        let target = Node::new("n2", "head", Point::new(320.0, 32.0));
        let edge = Edge::between("n1", "out", "n2", "in");
        let first = edge_wire_path(&[source.clone(), target.clone()], &edge).unwrap()[0];
        source.position = [48.0, 64.0];
        let second = edge_wire_path(&[source, target], &edge).unwrap()[0];
        assert!((second.from.x - first.from.x - 32.0).abs() < 0.1);
        assert!((second.from.y - first.from.y - 32.0).abs() < 0.1);
        assert_eq!(second.to, first.to);
    }

    #[test]
    fn a_backward_wire_does_not_swing_across_the_source() {
        let cubic = wire_cubic(Point::new(268.0, 80.0), Point::new(48.0, 300.0));
        assert!(cubic.c1.x - cubic.from.x < 48.0);
        assert!(cubic.c1.x < 320.0);
        assert!(cubic.c2.x < cubic.from.x);
    }

    #[test]
    fn detail_levels_follow_the_zoom_thresholds() {
        assert_eq!(detail_for(0.9), Detail::Full);
        assert_eq!(detail_for(0.55), Detail::Full);
        assert_eq!(detail_for(0.549), Detail::Compact);
        assert_eq!(detail_for(0.3), Detail::Compact);
        assert_eq!(detail_for(0.299), Detail::Title);
        assert_eq!(detail_for(0.05), Detail::Title);
    }

    #[test]
    fn a_point_on_the_cubic_hits_and_a_far_point_does_not() {
        let path = wire_path(Point::new(0.0, 0.0), &[], Point::new(200.0, 0.0));
        let mid = cubic_point(path[0], 0.5);
        assert_eq!(wire_hit(&path, mid, 6.0), Some(0));
        assert_eq!(wire_hit(&path, Point::new(mid.x, mid.y + 40.0), 6.0), None);
    }

    #[test]
    fn a_chain_with_reroutes_is_continuous_and_ends_where_it_should() {
        let from = Point::new(0.0, 0.0);
        let to = Point::new(400.0, 0.0);
        let bends = [Point::new(120.0, 120.0), Point::new(260.0, -80.0)];
        for reroutes in [Vec::new(), vec![bends[0]], vec![bends[0], bends[1]]] {
            let path = wire_path(from, &reroutes, to);
            assert_eq!(path.len(), reroutes.len() + 1);
            assert_eq!(path[0].from, from);
            assert_eq!(path[path.len() - 1].to, to);
            for pair in path.windows(2) {
                assert_eq!(pair[0].to, pair[1].from, "segments must share endpoints");
            }
        }
        // Each segment leaves horizontally, so a reroute is a real corner.
        let path = wire_path(from, &bends, to);
        assert!((path[0].c1.y - path[0].from.y).abs() < 0.001);
        assert!((path[2].c2.y - path[2].to.y).abs() < 0.001);
    }

    #[test]
    fn a_hit_on_a_chain_names_its_segment() {
        let bends = [Point::new(120.0, 160.0)];
        let path = wire_path(Point::new(0.0, 0.0), &bends, Point::new(400.0, 0.0));
        // Near the bend point itself, inside the second segment.
        let at = cubic_point(path[1], 0.5);
        assert_eq!(wire_hit(&path, at, 8.0), Some(1));
        let straight = wire_path(Point::new(0.0, 0.0), &[], Point::new(400.0, 0.0));
        let mid = cubic_point(straight[0], 0.5);
        assert_eq!(wire_hit(&straight, mid, 8.0), Some(0));
    }
}
