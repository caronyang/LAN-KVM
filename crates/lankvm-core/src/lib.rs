//! Shared geometry, layout, and protocol types for LAN-KVM.

use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn from_xywh(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        }
    }

    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub const fn contains(self, point: Point) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    pub fn center(self) -> Point {
        Point::new(self.left + self.width() / 2, self.top + self.height() / 2)
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{}+{},{}",
            self.width(),
            self.height(),
            self.left,
            self.top
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub const fn opposite(self) -> Self {
        match self {
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub rect: Rect,
    pub work_area: Rect,
    pub dpi: u32,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeLayout {
    pub id: String,
    pub name: String,
    pub monitors: Vec<MonitorInfo>,
    /// Position of the whole node in the shared logical canvas.
    pub canvas: Rect,
}

impl NodeLayout {
    pub fn bounds(&self) -> Rect {
        self.monitors
            .iter()
            .map(|m| m.rect)
            .fold(Rect::default(), |acc, rect| acc.union(rect))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub nodes: Vec<NodeLayout>,
}

impl Layout {
    pub fn find(&self, id: &str) -> Option<&NodeLayout> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Finds the node immediately adjacent to `from` on the given side.
    /// The canvas rectangles must touch (or overlap) on that side.
    pub fn neighbor(&self, from: &str, edge: Edge) -> Option<&NodeLayout> {
        let source = self.find(from)?;
        let s = source.canvas;
        self.nodes
            .iter()
            .filter(|candidate| candidate.id != from)
            .filter(|candidate| {
                let r = candidate.canvas;
                match edge {
                    Edge::Left => {
                        r.right >= s.left
                            && r.right <= s.left + 1
                            && r.top < s.bottom
                            && s.top < r.bottom
                    }
                    Edge::Right => {
                        r.left >= s.right - 1
                            && r.left <= s.right + 1
                            && r.top < s.bottom
                            && s.top < r.bottom
                    }
                    Edge::Top => {
                        r.bottom >= s.top
                            && r.bottom <= s.top + 1
                            && r.left < s.right
                            && s.left < r.right
                    }
                    Edge::Bottom => {
                        r.top >= s.bottom - 1
                            && r.top <= s.bottom + 1
                            && r.left < s.right
                            && s.left < r.right
                    }
                }
            })
            .min_by_key(|candidate| {
                let r = candidate.canvas;
                match edge {
                    Edge::Left => (s.left - r.right).abs(),
                    Edge::Right => (r.left - s.right).abs(),
                    Edge::Top => (s.top - r.bottom).abs(),
                    Edge::Bottom => (r.top - s.bottom).abs(),
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, w: i32, h: i32) -> MonitorInfo {
        MonitorInfo {
            id: 0,
            name: "test".into(),
            rect: Rect::from_xywh(x, y, w, h),
            work_area: Rect::from_xywh(x, y, w, h),
            dpi: 96,
            primary: false,
        }
    }

    fn node(id: &str, x: i32, y: i32, w: i32, h: i32) -> NodeLayout {
        NodeLayout {
            id: id.into(),
            name: id.into(),
            monitors: vec![monitor(x, y, w, h)],
            canvas: Rect::from_xywh(x, y, w, h),
        }
    }

    #[test]
    fn rect_geometry() {
        let r = Rect::from_xywh(-1920, 0, 1920, 1080);
        assert_eq!(r.width(), 1920);
        assert_eq!(r.height(), 1080);
        assert!(r.contains(Point::new(-1, 1)));
        assert!(!r.contains(Point::new(0, 1)));
        assert_eq!(r.center(), Point::new(-960, 540));
    }

    #[test]
    fn layout_neighbors() {
        let layout = Layout {
            nodes: vec![
                node("left", 0, 0, 1920, 1080),
                node("center", 1920, 0, 1920, 1080),
                node("right", 3840, 0, 1920, 1080),
            ],
        };
        assert_eq!(layout.neighbor("center", Edge::Left).unwrap().id, "left");
        assert_eq!(layout.neighbor("center", Edge::Right).unwrap().id, "right");
        assert!(layout.neighbor("left", Edge::Left).is_none());
        assert!(layout.neighbor("right", Edge::Right).is_none());
    }
}
