//! Viewer geometry. The picture is scaled to fit the window without
//! cropping and centered, with black bars on the other axis. Wire points are
//! normalized to the encoded canvas, top left first, like window coordinates.
use super::input::Content;
use extend_computer_agent::display::input::Point;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a `source`-sized picture sits inside `area`, or `None` for empty or
/// invalid sizes.
pub fn aspect_fit(area: Area, source: (f64, f64)) -> Option<Area> {
    let (width, height) = source;
    if ![area.x, area.y, area.width, area.height, width, height]
        .iter()
        .all(|n| n.is_finite())
        || area.width <= 0.0
        || area.height <= 0.0
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    let scale = (area.width / width).min(area.height / height);
    let (fitted_width, fitted_height) = (width * scale, height * scale);
    Some(Area {
        x: area.x + (area.width - fitted_width) / 2.0,
        y: area.y + (area.height - fitted_height) / 2.0,
        width: fitted_width,
        height: fitted_height,
    })
}

/// The fitted picture in whole pixels of a `width` x `height` target, for
/// the video processor's destination rectangle: (left, top, right, bottom).
pub fn pixel_rect(width: u32, height: u32, source: (u32, u32)) -> Option<(i32, i32, i32, i32)> {
    let fitted = aspect_fit(
        Area {
            x: 0.0,
            y: 0.0,
            width: f64::from(width),
            height: f64::from(height),
        },
        (f64::from(source.0), f64::from(source.1)),
    )?;
    let left = fitted.x.round() as i32;
    let top = fitted.y.round() as i32;
    let right = ((fitted.x + fitted.width).round() as i32).clamp(left + 1, width as i32);
    let bottom = ((fitted.y + fitted.height).round() as i32).clamp(top + 1, height as i32);
    Some((left, top, right, bottom))
}

/// A window position as a canvas point, when it lands on the picture and
/// inside the source's captured `content`.
pub fn point(
    location: (f64, f64),
    viewport: Area,
    canvas: (f64, f64),
    content: &Content,
) -> Option<Point> {
    if !location.0.is_finite() || !location.1.is_finite() {
        return None;
    }
    let picture = aspect_fit(viewport, canvas)?;
    let point = Point {
        x: (location.0 - picture.x) / picture.width,
        y: (location.1 - picture.y) / picture.height,
    };
    ((0.0..=1.0).contains(&point.x) && (0.0..=1.0).contains(&point.y) && content.contains(point))
        .then_some(point)
}

/// A wheel delta in macOS pixels. One 120-unit notch is 10 pixels, as in the
/// input helper; a positive horizontal delta (tilt right) scrolls left.
pub fn wheel_pixels(delta: i16, horizontal: bool) -> (f64, f64) {
    let pixels = f64::from(delta) / 12.0;
    if horizontal {
        (-pixels, 0.0)
    } else {
        (0.0, pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: f64, y: f64, width: f64, height: f64) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn fits_wide_and_tall_pictures_with_centered_bars() {
        assert_eq!(
            aspect_fit(area(0., 0., 1920., 1080.), (1440., 900.)),
            Some(area(96., 0., 1728., 1080.))
        );
        assert_eq!(
            aspect_fit(area(10., 20., 1000., 1000.), (1000., 500.)),
            Some(area(10., 270., 1000., 500.))
        );
        assert_eq!(aspect_fit(area(0., 0., 0., 10.), (10., 10.)), None);
        assert_eq!(aspect_fit(area(0., 0., 10., 10.), (f64::NAN, 10.)), None);
        assert_eq!(
            pixel_rect(1920, 1080, (1440, 900)),
            Some((96, 0, 1824, 1080))
        );
        assert_eq!(
            pixel_rect(1920, 1080, (1920, 1080)),
            Some((0, 0, 1920, 1080))
        );
        assert_eq!(
            pixel_rect(1080, 1920, (1920, 1080)),
            Some((0, 656, 1080, 1264))
        );
        assert_eq!(pixel_rect(100, 100, (0, 10)), None);
    }

    #[test]
    fn points_map_through_the_letterbox_and_reject_bars_and_content_edges() {
        let viewport = area(10., 20., 1000., 1000.);
        let canvas = (1000., 500.);
        let full = Content::default();
        assert_eq!(
            point((510., 520.), viewport, canvas, &full),
            Some(Point { x: 0.5, y: 0.5 })
        );
        assert_eq!(
            point((10., 770.), viewport, canvas, &full),
            Some(Point { x: 0., y: 1. })
        );
        for outside in [(510., 100.), (f64::NAN, 400.), (1011., 400.), (510., 771.)] {
            assert_eq!(point(outside, viewport, canvas, &full), None, "{outside:?}");
        }
        let inset = Content {
            x: 0.1,
            y: 0.2,
            width: 0.8,
            height: 0.6,
        };
        assert_eq!(point((10., 520.), viewport, canvas, &inset), None);
        assert_eq!(
            point((510., 520.), viewport, canvas, &inset),
            Some(Point { x: 0.5, y: 0.5 })
        );
        assert_eq!(point((0., 0.), area(0., 0., 0., 0.), canvas, &full), None);
    }

    #[test]
    fn wheel_notches_are_ten_pixels_and_tilt_right_scrolls_left() {
        assert_eq!(wheel_pixels(120, false), (0., 10.));
        assert_eq!(wheel_pixels(-240, false), (0., -20.));
        assert_eq!(wheel_pixels(120, true), (-10., 0.));
        assert_eq!(wheel_pixels(30, false), (0., 2.5));
    }
}
