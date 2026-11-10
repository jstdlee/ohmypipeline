//! The node canvas: cards, ports, wires, and the camera.
//!
//! Connection rules live in `graph`. This crate turns a pointer into those calls
//! and paints the result.

mod layout;
mod model;
mod motion;
mod plot;

pub use layout::{
    Cubic, Detail, PortSlot, Side, card_height, card_rect, cubic_point, detail_for, edge_wire_path,
    port_slots, to_local, wire_cubic, wire_hit, wire_path,
};
pub use model::{CanvasEvent, CanvasModel, Dot, FrameView, Tool};
pub use plot::{PlotPoint, feature_chart, legend_entries, points_for_encodings, points_from};
