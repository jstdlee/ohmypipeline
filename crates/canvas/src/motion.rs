//! Motion for the canvas.
//!
//! One curve, a few durations. Hover and press stay near 140 ms. A card
//! settles in 180 ms. The camera eases in 280 ms. Reduced motion, from the
//! in-app toggle or the desktop preference, skips the travel and keeps the
//! state change.

use std::time::Duration;

#[allow(dead_code)]
pub const HOVER: Duration = Duration::from_millis(140);
pub const SETTLE: Duration = Duration::from_millis(180);
pub const ENTER: Duration = Duration::from_millis(220);
pub const CAMERA: Duration = Duration::from_millis(280);
/// A frame collapsing or expanding.
pub const COLLAPSE: Duration = Duration::from_millis(180);
/// Auto-layout travel. A longer settle reads as a re-arrangement, not a drop.
pub const LAYOUT: Duration = Duration::from_millis(260);
/// The minimap fade.
pub const FADE: Duration = Duration::from_millis(140);

/// `cubic-bezier(0.22, 1, 0.36, 1)` approximated as a fast ease-out.
/// Interrupt a running animation by replacing its target; this curve has no loop.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * ease_out(t)
}
