//! Pure input logic: edge detection and cross-screen coordinate mapping.
//!
//! This crate deliberately has no OS dependencies so it can be tested on any
//! platform. The Windows backend feeds it raw mouse deltas and the current
//! cursor position.

use std::cmp::Ordering;

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

    /// How far the outward push has accumulated inside the current window.
    pub fn accumulated(&self) -> f32 {
        self.accumulated
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

/// Detects the deliberate outward push that hands control back to the peer.
///
/// The client used to keep one accumulator for the whole time it controlled
/// the peer (sum of every motion delta since the switch). That sum drifts away
/// from the real pointer: injected motion is accelerated and clamped at the
/// screen edges, so a push at the far edge keeps adding while the cursor is
/// already stuck. Moving back to the entry edge then "spends" the drift, and
/// every retry there starts from the same value — after operating on the peer
/// for a while, crossing back needs a single push larger than the drift, so it
/// only works occasionally.
///
/// This tracker only counts the push that happens while the pointer is sitting
/// on the entry edge, and clears on any inward move, so a retry always starts
/// from zero.
#[derive(Debug, Clone)]
pub struct ReturnPush {
    threshold_px: i32,
    pushed: i32,
    hits: u32,
}

impl ReturnPush {
    pub fn new(threshold_px: i32) -> Self {
        Self {
            threshold_px: threshold_px.max(1),
            pushed: 0,
            hits: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pushed = 0;
        self.hits = 0;
    }

    /// How far the pointer has been pushed out through the entry edge in the
    /// current attempt.
    pub fn pushed(&self) -> i32 {
        self.pushed
    }

    /// Call once per motion packet.
    ///
    /// `at_edge` is true when the pointer sits on the entry edge and
    /// `outward_px` is the signed movement across it: positive points at the
    /// other screen, negative points into this one, zero is vertical only.
    /// Returns true once the push is deliberate (at least two consecutive
    /// outward packets and `threshold_px` in total).
    pub fn update(&mut self, at_edge: bool, outward_px: i32) -> bool {
        if !at_edge {
            self.reset();
            return false;
        }
        match outward_px.cmp(&0) {
            Ordering::Greater => {
                self.pushed = self.pushed.saturating_add(outward_px);
                self.hits += 1;
            }
            Ordering::Less => {
                // Moving back into this screen cancels the attempt.
                self.reset();
            }
            Ordering::Equal => {}
        }
        self.hits >= 2 && self.pushed >= self.threshold_px
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
    fn return_push_needs_two_packets() {
        let mut p = ReturnPush::new(10);
        assert!(!p.update(true, 100));
        assert!(p.update(true, 1));
    }

    #[test]
    fn return_push_clears_when_leaving_the_edge() {
        let mut p = ReturnPush::new(30);
        // A long session leaves no credit behind.
        assert!(!p.update(false, 5000));
        assert!(!p.update(true, 20));
        // Leaving and coming back starts the attempt again, but the old
        // 20 px must not count.
        assert!(!p.update(false, 0));
        assert!(!p.update(true, 20));
        assert!(p.update(true, 10));
    }

    #[test]
    fn return_push_inward_motion_cancels() {
        let mut p = ReturnPush::new(10);
        assert!(!p.update(true, 8));
        assert!(!p.update(true, -5));
        assert!(!p.update(true, 8));
        assert!(p.update(true, 3));
    }

    #[test]
    fn return_push_vertical_motion_keeps_the_attempt() {
        let mut p = ReturnPush::new(10);
        assert!(!p.update(true, 6));
        assert!(!p.update(true, 0));
        assert!(p.update(true, 6));
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
