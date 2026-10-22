//! Pure input logic: edge detection and cross-screen coordinate mapping.
//!
//! This crate deliberately has no OS dependencies so it can be tested on any
//! platform. The Windows backend feeds it raw mouse deltas and the current
//! cursor position.

use lankvm_core::{Edge, Point, Rect};

/// Detects an intentional "push through the edge" gesture.
#[derive(Debug, Clone)]
pub struct EdgeDetector {
    edge: Edge,
    threshold_px: f32,
    window_ms: u64,
    accumulated: f32,
    last_ms: Option<u64>,
    fired: bool,
}

impl EdgeDetector {
    pub fn new(edge: Edge, threshold_px: f32, window_ms: u64) -> Self {
        Self {
            edge,
            threshold_px: threshold_px.max(1.0),
            window_ms,
            accumulated: 0.0,
            last_ms: None,
            fired: false,
        }
    }

    pub fn edge(&self) -> Edge {
        self.edge
    }

    pub fn set_edge(&mut self, edge: Edge) {
        self.edge = edge;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.accumulated = 0.0;
        self.last_ms = None;
        self.fired = false;
    }

    /// Returns `true` exactly once when the threshold is crossed.
    ///
    /// `dx`/`dy` are the raw (unaccelerated) deltas since the previous sample.
    /// `now_ms` must be monotonic (e.g. `Instant` elapsed milliseconds).
    pub fn update(&mut self, cursor: Point, bounds: Rect, dx: f32, dy: f32, now_ms: u64) -> bool {
        if self.fired {
            return false;
        }

        if !at_edge(cursor, bounds, self.edge) {
            self.accumulated = 0.0;
            self.last_ms = Some(now_ms);
            return false;
        }

        let outward = match self.edge {
            Edge::Left => -dx,
            Edge::Right => dx,
            Edge::Top => -dy,
            Edge::Bottom => dy,
        };

        if outward <= 0.0 {
            self.accumulated = 0.0;
            self.last_ms = Some(now_ms);
            return false;
        }

        if let Some(last) = self.last_ms {
            if now_ms.saturating_sub(last) > self.window_ms {
                self.accumulated = 0.0;
            }
        }
        self.last_ms = Some(now_ms);
        self.accumulated += outward;

        if self.accumulated >= self.threshold_px {
            self.fired = true;
            return true;
        }
        false
    }
}

pub fn at_edge(cursor: Point, bounds: Rect, edge: Edge) -> bool {
    match edge {
        Edge::Left => cursor.x <= bounds.left,
        Edge::Right => cursor.x >= bounds.right - 1,
        Edge::Top => cursor.y <= bounds.top,
        Edge::Bottom => cursor.y >= bounds.bottom - 1,
    }
}

/// Maps the cursor's position along `source` to the corresponding position on
/// `target`, clamped to the target's edges.
///
/// For a horizontal edge crossing, the vertical position is preserved
/// proportionally; for a vertical crossing, the horizontal position is.
pub fn map_entry(source: Rect, target: Rect, cursor: Point, edge: Edge) -> Point {
    match edge {
        Edge::Left | Edge::Right => {
            let t = ratio(cursor.y - source.top, source.height());
            let y = target.top + (t * target.height() as f32).round() as i32;
            let x = if edge == Edge::Left {
                target.left
            } else {
                target.right - 1
            };
            Point::new(x, clamp(y, target.top, target.bottom - 1))
        }
        Edge::Top | Edge::Bottom => {
            let t = ratio(cursor.x - source.left, source.width());
            let x = target.left + (t * target.width() as f32).round() as i32;
            let y = if edge == Edge::Top {
                target.top
            } else {
                target.bottom - 1
            };
            Point::new(clamp(x, target.left, target.right - 1), y)
        }
    }
}

fn ratio(offset: i32, length: i32) -> f32 {
    if length <= 1 {
        return 0.0;
    }
    (offset as f32 / (length - 1) as f32).clamp(0.0, 1.0)
}

fn clamp(value: i32, min: i32, max: i32) -> i32 {
    value.max(min).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::from_xywh(0, 0, 1920, 1080)
    }

    #[test]
    fn right_edge_requires_outward_push() {
        let mut d = EdgeDetector::new(Edge::Right, 20.0, 300);
        assert!(!d.update(Point::new(1919, 500), bounds(), 5.0, 0.0, 0));
        assert!(!d.update(Point::new(1919, 500), bounds(), 5.0, 0.0, 10));
        assert!(d.update(Point::new(1919, 500), bounds(), 10.0, 0.0, 20));
        // Fires only once until reset.
        assert!(!d.update(Point::new(1919, 500), bounds(), 10.0, 0.0, 30));
        d.reset();
        assert!(d.update(Point::new(1919, 500), bounds(), 20.0, 0.0, 40));
    }

    #[test]
    fn inward_motion_resets() {
        let mut d = EdgeDetector::new(Edge::Right, 20.0, 300);
        assert!(!d.update(Point::new(1919, 500), bounds(), 15.0, 0.0, 0));
        assert!(!d.update(Point::new(1919, 500), bounds(), -15.0, 0.0, 10));
        assert!(!d.update(Point::new(1919, 500), bounds(), 10.0, 0.0, 20));
    }

    #[test]
    fn leaving_edge_resets() {
        let mut d = EdgeDetector::new(Edge::Right, 20.0, 300);
        assert!(!d.update(Point::new(1919, 500), bounds(), 15.0, 0.0, 0));
        assert!(!d.update(Point::new(1800, 500), bounds(), 15.0, 0.0, 10));
        assert!(!d.update(Point::new(1919, 500), bounds(), 10.0, 0.0, 20));
    }

    #[test]
    fn window_expiry_resets() {
        let mut d = EdgeDetector::new(Edge::Right, 20.0, 100);
        assert!(!d.update(Point::new(1919, 500), bounds(), 15.0, 0.0, 0));
        // 200 ms later: the previous 15 px must not count.
        assert!(!d.update(Point::new(1919, 500), bounds(), 10.0, 0.0, 200));
    }

    #[test]
    fn entry_mapping_preserves_ratio() {
        let source = Rect::from_xywh(0, 0, 1920, 1080);
        let target = Rect::from_xywh(0, 0, 2560, 1440);
        let p = map_entry(source, target, Point::new(1919, 540), Edge::Right);
        // 540 / 1079 ≈ 0.5005 → 720 on the target
        assert_eq!(p.x, 2559);
        assert!((p.y - 720).abs() <= 1);
    }

    #[test]
    fn entry_mapping_clamps() {
        let source = Rect::from_xywh(0, 0, 1920, 1080);
        let target = Rect::from_xywh(0, 0, 1920, 1080);
        assert_eq!(
            map_entry(source, target, Point::new(0, 5000), Edge::Left).y,
            1079
        );
        assert_eq!(
            map_entry(source, target, Point::new(0, -100), Edge::Left).y,
            0
        );
    }
}
