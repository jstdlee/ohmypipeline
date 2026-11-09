//! The node canvas: cards, ports, wires, and the camera.
//!
//! Connection rules live in `graph`. This crate turns a pointer into those calls
//! and paints the result.

mod layout;
mod model;
mod motion;

pub use layout::{
    Cubic, PortSlot, Side, card_height, card_rect, cubic_point, port_slots, to_local, wire_cubic,
    wire_hit,
};
pub use model::{CanvasEvent, CanvasModel, Dot, Tool};
