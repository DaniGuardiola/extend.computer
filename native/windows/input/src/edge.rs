//! Edge crossing between this display and the peer's, a port of
//! native/macos/Input/EdgeLayout.swift. Coordinates are logical display points,
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
    use serde_json::Value;

    /// Fixture numbers, with non-finite values written as strings.
    fn number(value: &Value) -> f64 {
        match value.as_str() {
            Some("nan") => f64::NAN,
            Some("inf") => f64::INFINITY,
            Some("-inf") => f64::NEG_INFINITY,
            _ => value.as_f64().unwrap(),
        }
    }

    fn layout(value: &Value) -> Result<Layout> {
        let size = |key: &str, index: usize| number(&value[key][index]);
        let side = Side::parse(value["side"].as_str().unwrap()).unwrap();
        Layout::new(
            size("local", 0),
            size("local", 1),
            size("remote", 0),
            size("remote", 1),
            side,
            number(&value["offset_y"]),
        )
    }

    /// Plays one step; side changes have no outcome.
    fn play(l: &mut Layout, step: &Value) -> Option<Motion> {
        if let Some(side) = step["set_side"].as_str() {
            l.set_side(Side::parse(side).unwrap());
            return None;
        }
        if step["cancel"] == true {
            return Some(l.cancel().map_or(Motion::Local, Motion::Returned));
        }
        let m: Vec<f64> = step["motion"]
            .as_array()
            .unwrap()
            .iter()
            .map(number)
            .collect();
        Some(l.motion(
            Point { x: m[0], y: m[1] },
            m[2],
            m[3],
            step["exposed"].as_bool().unwrap_or(true),
            step["allow_return"].as_bool().unwrap_or(true),
        ))
    }

    fn expected(step: &Value) -> Motion {
        let at = || Point {
            x: number(&step["at"][0]),
            y: number(&step["at"][1]),
        };
        match step["expect"].as_str().unwrap() {
            "local" => Motion::Local,
            "remote" => Motion::Remote(at()),
            "returned" => Motion::Returned(at()),
            other => panic!("unknown outcome {other}"),
        }
    }

    fn close(a: &Motion, b: &Motion) -> bool {
        match (a, b) {
            (Motion::Remote(a), Motion::Remote(b)) | (Motion::Returned(a), Motion::Returned(b)) => {
                (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
            }
            _ => a == b,
        }
    }

    /// native/macos/tests/LayoutTests.swift replays the same file through the
    /// original EdgeLayout.swift.
    #[test]
    fn replays_shared_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../shared/fixtures/edge-layout.json"))
                .unwrap();
        for value in fixture["invalid_layouts"].as_array().unwrap() {
            assert!(layout(value).is_err(), "accepted {value}");
        }
        for case in fixture["cases"].as_array().unwrap() {
            let mut l = layout(&case["layout"]).unwrap();
            for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
                let name = format!("{} step {index}", case["name"]);
                if let Some(outcome) = play(&mut l, step) {
                    let expected = expected(step);
                    assert!(
                        close(&outcome, &expected),
                        "{name}: {outcome:?}, expected {expected:?}"
                    );
                }
                if let Some(side) = step["side"].as_str() {
                    assert_eq!(Some(l.side), Side::parse(side), "{name}");
                }
            }
        }
    }
}
