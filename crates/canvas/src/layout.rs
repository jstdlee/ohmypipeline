//! Card and wire geometry in canvas pixels.
//!
//! The file stores node positions, not curves. A wire's ends are the port
//! slots of the cards it joins, so moving a card moves the wire.

use graph::{Camera, Edge, Node, Point, card_width};

pub const PORT_ROW: f32 = 22.0;
pub const HEADER: f32 = 28.0;
pub const SUMMARY: f32 = 16.0;
pub const PAD_Y: f32 = 8.0;

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

pub fn wire_hit(cubic: Cubic, point: Point, radius: f32) -> bool {
    let mut closest = f32::MAX;
    for step in 0..=24 {
        let at = cubic_point(cubic, step as f32 / 24.0);
        let dx = at.x - point.x;
        let dy = at.y - point.y;
        closest = closest.min(dx * dx + dy * dy);
    }
    closest <= radius * radius
}

pub fn edge_cubic(nodes: &[Node], edge: &Edge) -> Option<Cubic> {
    let source = nodes.iter().find(|node| node.id == edge.from)?;
    let target = nodes.iter().find(|node| node.id == edge.to)?;
    let source_slots = port_slots(source);
    let target_slots = port_slots(target);
    let from = slot(&source_slots, &edge.from_port, Side::Out)?;
    let to = slot(&target_slots, &edge.to_port, Side::In)?;
    Some(wire_cubic(from.center, to.center))
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
        let first = edge_cubic(&[source.clone(), target.clone()], &edge).unwrap();
        source.position = [48.0, 64.0];
        let second = edge_cubic(&[source, target], &edge).unwrap();
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
    fn a_point_on_the_cubic_hits_and_a_far_point_does_not() {
        let cubic = wire_cubic(Point::new(0.0, 0.0), Point::new(200.0, 0.0));
        let mid = cubic_point(cubic, 0.5);
        assert!(wire_hit(cubic, mid, 6.0));
        assert!(!wire_hit(cubic, Point::new(mid.x, mid.y + 40.0), 6.0));
    }
}
