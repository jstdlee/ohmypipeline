use serde::{Deserialize, Serialize};

/// Canvas pixels between grid lines at zoom 1.
pub const GRID_PX: f32 = 16.0;

/// Window pixels. Measured after the camera, so zoom does not change the grab.
pub const SNAP_RADIUS_PX: f32 = 12.0;

const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 2.5;

/// `window = canvas * zoom + camera`. `x` and `y` are the window position of the canvas origin.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn to_window(self, canvas: Point) -> Point {
        Point {
            x: canvas.x * self.zoom + self.x,
            y: canvas.y * self.zoom + self.y,
        }
    }

    pub fn to_canvas(self, window: Point) -> Point {
        Point {
            x: (window.x - self.x) / self.zoom,
            y: (window.y - self.y) / self.zoom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.w && p.y < self.y + self.h
    }

    pub fn union(self, other: Self) -> Self {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.w).max(other.x + other.w);
        let y1 = (self.y + self.h).max(other.y + other.h);
        Self {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }
}

pub fn point_in_rect(p: Point, rect: Rect) -> bool {
    rect.contains(p)
}

/// Snap a canvas point to the 16 px grid.
pub fn snap_grid(p: Point) -> Point {
    Point {
        x: (p.x / GRID_PX).round() * GRID_PX,
        y: (p.y / GRID_PX).round() * GRID_PX,
    }
}

/// Zoom toward the pointer so the canvas point under it stays put.
pub fn zoom_toward(camera: Camera, pointer_window: Point, factor: f32) -> Camera {
    let canvas = camera.to_canvas(pointer_window);
    let zoom = (camera.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    Camera {
        zoom,
        x: pointer_window.x - canvas.x * zoom,
        y: pointer_window.y - canvas.y * zoom,
    }
}

/// Frame `bounds` (canvas px) inside a view of `view_w` by `view_h` (window px).
pub fn fit_camera(view_w: f32, view_h: f32, bounds: Rect, pad: f32) -> Camera {
    if bounds.w <= 1.0 || bounds.h <= 1.0 || view_w <= pad * 2.0 || view_h <= pad * 2.0 {
        return Camera {
            x: pad - bounds.x,
            y: pad - bounds.y,
            zoom: 1.0,
        };
    }
    let zoom = ((view_w - pad * 2.0) / bounds.w)
        .min((view_h - pad * 2.0) / bounds.h)
        .clamp(ZOOM_MIN, ZOOM_MAX);
    let cx = bounds.x + bounds.w * 0.5;
    let cy = bounds.y + bounds.h * 0.5;
    Camera {
        zoom,
        x: view_w * 0.5 - cx * zoom,
        y: view_h * 0.5 - cy * zoom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_sixteen_canvas_pixels() {
        let p = snap_grid(Point::new(20.0, 7.0));
        assert_eq!(p, Point::new(16.0, 0.0));
        assert_eq!(GRID_PX, 16.0);
    }

    #[test]
    fn zoom_keeps_the_canvas_point_under_the_pointer() {
        let start = Camera {
            x: 10.0,
            y: -4.0,
            zoom: 1.0,
        };
        let pointer = Point::new(80.0, 40.0);
        let canvas = start.to_canvas(pointer);
        let next = zoom_toward(start, pointer, 2.0);
        let back = next.to_canvas(pointer);
        assert!((back.x - canvas.x).abs() < 1e-4);
        assert!((back.y - canvas.y).abs() < 1e-4);
        assert!((next.zoom - 2.0).abs() < 1e-4);
    }

    #[test]
    fn fit_puts_the_bounds_inside_the_view() {
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        let camera = fit_camera(800.0, 400.0, bounds, 20.0);
        let origin = camera.to_window(Point::new(bounds.x, bounds.y));
        let far = camera.to_window(Point::new(bounds.x + bounds.w, bounds.y + bounds.h));
        assert!(origin.x >= 19.0 && origin.y >= 19.0);
        assert!(far.x <= 781.0 && far.y <= 381.0);
    }

    #[test]
    fn fit_zooms_out_to_a_tenth_for_a_graph_that_would_not_fit() {
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            w: 8000.0,
            h: 4000.0,
        };
        let camera = fit_camera(800.0, 600.0, bounds, 72.0);
        assert!((camera.zoom - 0.1).abs() < 1e-4);
    }
}
