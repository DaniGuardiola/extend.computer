//! Edge crossing between this display and the peer's, a port of
//! native/macos/EdgeLayout.swift. Coordinates are logical display points,
//! not pixels. The remote top is `offset_y` points below the local top.
use anyhow::{ensure, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Motion {
    Local,
    /// Normalized position on the remote display.
    Remote(Point),
    /// Local point to put the pointer back at.
    Returned(Point),
}

pub struct Layout {
    pub local_width: f64,
    pub local_height: f64,
    pub remote_width: f64,
    pub remote_height: f64,
    pub side: Side,
    pending_side: Option<Side>,
    offset_y: f64,
    remote: Option<Point>,
    armed: bool,
}

impl Layout {
    pub fn new(
        local_width: f64,
        local_height: f64,
        remote_width: f64,
        remote_height: f64,
        side: Side,
        offset_y: f64,
    ) -> Result<Self> {
        let sizes = [local_width, local_height, remote_width, remote_height];
        ensure!(
            sizes
                .iter()
                .all(|s| s.is_finite() && (16.0..=32768.0).contains(s))
                && offset_y.is_finite()
                && offset_y.max(0.0) < local_height.min(offset_y + remote_height),
            "Displays need valid sizes and an overlapping vertical edge."
        );
        Ok(Self {
            local_width,
            local_height,
            remote_width,
            remote_height,
            side,
            pending_side: None,
            offset_y,
            remote: None,
            armed: true,
        })
    }

    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    /// Keeps the return route while remote; the change applies on return.
    pub fn set_side(&mut self, side: Side) {
        if self.remote.is_some() {
            self.pending_side = Some(side);
        } else {
            self.side = side;
            self.pending_side = None;
        }
    }

    fn apply_pending_side(&mut self) {
        if let Some(side) = self.pending_side.take() {
            self.side = side;
        }
    }

    pub fn motion(
        &mut self,
        local: Point,
        dx: f64,
        dy: f64,
        exposed: bool,
        allow_return: bool,
    ) -> Motion {
        if ![local.x, local.y, dx, dy].iter().all(|v| v.is_finite()) {
            return Motion::Local;
        }
        let left = self.side == Side::Left;
        if let Some(mut point) = self.remote {
            point.x += dx;
            point.y = (point.y + dy).clamp(0.0, self.remote_height - 1.0);
            let local_y = point.y + self.offset_y;
            let returning = if left {
                point.x >= self.remote_width
            } else {
                point.x < 0.0
            };
            if allow_return && returning && local_y >= 0.0 && local_y < self.local_height {
                self.remote = None;
                self.armed = false;
                let x = if left { 4.0 } else { self.local_width - 5.0 };
                self.apply_pending_side();
                return Motion::Returned(Point { x, y: local_y });
            }
            point.x = point.x.clamp(0.0, self.remote_width - 1.0);
            self.remote = Some(point);
            return Motion::Remote(self.normalized(point));
        }
        let away = if left {
            local.x > 6.0
        } else {
            local.x < self.local_width - 7.0
        };
        if !self.armed {
            self.armed = away;
            return Motion::Local;
        }
        let outward = if left {
            local.x <= 1.0 && dx < 0.0
        } else {
            local.x >= self.local_width - 2.0 && dx > 0.0
        };
        if !exposed
            || !outward
            || local.y < 0.0
            || local.y >= self.local_height
            || local.y < self.offset_y
            || local.y >= self.offset_y + self.remote_height
        {
            return Motion::Local;
        }
        let point = Point {
            x: if left { self.remote_width - 1.0 } else { 0.0 },
            y: local.y - self.offset_y,
        };
        self.remote = Some(point);
        Motion::Remote(self.normalized(point))
    }

    /// Ends a remote stay early; returns where to put the local pointer.
    pub fn cancel(&mut self) -> Option<Point> {
        let point = self.remote.take()?;
        self.armed = false;
        Some(Point {
            x: if self.side == Side::Left {
                4.0
            } else {
                self.local_width - 5.0
            },
            y: (point.y + self.offset_y).clamp(0.0, self.local_height - 1.0),
        })
    }

    fn normalized(&self, point: Point) -> Point {
        Point {
            x: point.x / (self.remote_width - 1.0),
            y: point.y / (self.remote_height - 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(side: Side) -> Layout {
        Layout::new(1000.0, 800.0, 1201.0, 801.0, side, 0.0).unwrap()
    }

    #[test]
    fn crosses_left_and_returns_on_the_far_side() {
        let mut l = layout(Side::Left);
        let at = Point { x: 1.0, y: 400.0 };
        assert_eq!(l.motion(at, 3.0, 0.0, true, true), Motion::Local);
        assert_eq!(
            l.motion(at, -3.0, 0.0, true, true),
            Motion::Remote(Point { x: 1.0, y: 0.5 })
        );
        assert_eq!(
            l.motion(at, -600.0, 0.0, true, true),
            Motion::Remote(Point { x: 0.5, y: 0.5 })
        );
        assert_eq!(
            l.motion(at, 700.0, 0.0, true, true),
            Motion::Returned(Point { x: 4.0, y: 400.0 })
        );
        // Re-entry waits until the pointer has moved away from the edge.
        assert_eq!(l.motion(at, -3.0, 0.0, true, true), Motion::Local);
        assert_eq!(
            l.motion(Point { x: 10.0, y: 400.0 }, 0.0, 0.0, true, true),
            Motion::Local
        );
        assert!(matches!(
            l.motion(at, -3.0, 0.0, true, true),
            Motion::Remote(_)
        ));
    }

    #[test]
    fn right_edge_hidden_edges_and_held_buttons_stay_local() {
        let mut l = layout(Side::Right);
        let at = Point { x: 998.0, y: 10.0 };
        assert_eq!(l.motion(at, 5.0, 0.0, false, true), Motion::Local);
        assert!(matches!(
            l.motion(at, 5.0, 0.0, true, true),
            Motion::Remote(_)
        ));
        assert!(matches!(
            l.motion(at, -50.0, 0.0, true, false),
            Motion::Remote(_)
        ));
        assert_eq!(
            l.motion(at, -50.0, 0.0, true, true),
            Motion::Returned(Point { x: 995.0, y: 10.0 })
        );
    }

    #[test]
    fn side_changes_wait_for_return() {
        let mut l = layout(Side::Left);
        l.motion(Point { x: 0.0, y: 5.0 }, -1.0, 0.0, true, true);
        l.set_side(Side::Right);
        assert_eq!(l.side, Side::Left);
        assert_eq!(l.cancel(), Some(Point { x: 4.0, y: 5.0 }));
        assert!(Layout::new(10.0, 800.0, 1200.0, 800.0, Side::Left, 0.0).is_err());
    }
}
