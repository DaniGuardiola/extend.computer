//! The extended display's modes, identity and place on the desktop, without
//! touching Windows.

/// Monitors without a seed from the app share this one, so they still map to
/// a single entry in Windows' display database.
pub const DEFAULT_SERIAL: u32 = 0x4543_0001;
/// The app's other stream sizes, in points: `WxH,WxH`. The macOS helper reads
/// the same list for its virtual display.
pub const PRESETS: &str = "EXTEND_DISPLAY_PRESETS";
/// The monitor's EDID serial, a decimal `u32` that is stable per peer.
pub const SERIAL: &str = "EXTEND_DISPLAY_SERIAL";
const MAX_PRESETS: usize = 8;
/// The monitor's preferred mode, which its EDID carries with the physical
/// size and the serial. Windows keys its display database (arrangement,
/// mode, scale) on the EDID, so both stay the same whatever the stream size:
/// each peer's display keeps one identity per frame rate. The stream size
/// and presets are further modes.
pub const PREFERRED: (u32, u32) = (1920, 1080);
/// The driver takes eight modes beside the preferred one.
const MAX_FALLBACKS: usize = 8;
/// Windows' display scale steps, in percent.
const SCALES: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

/// A rectangle in desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.width as i32
    }
    fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }
    fn overlaps_rows(&self, other: &Rect) -> bool {
        self.y < other.bottom() && other.y < self.bottom()
    }
    fn overlaps_columns(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right()
    }
    /// Whether the two share any pixel. Monitors that extend the desktop
    /// never do; a duplicate of another monitor does.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.overlaps_rows(other) && self.overlaps_columns(other)
    }
    /// Whether the two share part of an edge, as neighbouring monitors do.
    fn touches(&self, other: &Rect) -> bool {
        ((self.x == other.right() || self.right() == other.x) && self.overlaps_rows(other))
            || ((self.y == other.bottom() || self.bottom() == other.y)
                && self.overlaps_columns(other))
    }
}

/// Up to eight `WxH` sizes from [`PRESETS`]; malformed entries are skipped.
pub fn presets(value: Option<&str>) -> Vec<(u32, u32)> {
    value
        .unwrap_or_default()
        .split(',')
        .take(MAX_PRESETS)
        .filter_map(|entry| {
            let (width, height) = entry.trim().split_once('x')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })
        .collect()
}

/// The serial from [`SERIAL`], or [`DEFAULT_SERIAL`] when it is missing,
/// malformed or zero (an EDID serial of zero means none).
pub fn serial(value: Option<&str>) -> u32 {
    value
        .and_then(|value| value.trim().parse().ok())
        .filter(|&seed| seed != 0)
        .unwrap_or(DEFAULT_SERIAL)
}

/// The monitor's modes in pixels: [`PREFERRED`] first, then `size`, the
/// stream size, then each preset the app may switch to, as the macOS helper
/// offers them. Presets are in points, so `scale` (2 for Retina) multiplies
/// them.
pub fn modes(size: (u32, u32), scale: u32, presets: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut modes = vec![PREFERRED];
    let presets = presets
        .iter()
        .take(MAX_PRESETS)
        .filter(|&&(width, height)| {
            width >= 640 && height >= 480 && width * scale <= 4096 && height * scale <= 4096
        })
        .map(|&(width, height)| (even(width * scale), even(height * scale)));
    for mode in std::iter::once(size).chain(presets) {
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes.truncate(1 + MAX_FALLBACKS);
    modes
}

/// The refresh rate, in Hz, of a monitor at `current` that streams at
/// `fps`: a faster frame rate needs a faster monitor, which replaces it; a
/// slower one only paces encoding, so the monitor stays where it is.
pub fn refresh_rate(current: u32, fps: u32) -> u32 {
    current.max(fps)
}

/// Encoded sizes stay even for 4:2:0 video.
pub fn even(value: u32) -> u32 {
    value & !1
}

/// The physical size, in millimetres, of `points` at 96 points per inch, the
/// size Windows expects at 100% scale.
pub fn millimetres(points: u32) -> u32 {
    (points * 254 + 480) / 960
}

/// The scale request for `percent`: steps from the scale Windows recommends,
/// which is `-min` steps above 100%, kept within `min..=max`. `None` if
/// `percent` isn't a Windows scale.
pub fn scale_step(percent: u32, min: i32, max: i32) -> Option<i32> {
    let index = SCALES.iter().position(|&scale| scale == percent)? as i32;
    (min <= max).then(|| (index + min).clamp(min, max))
}

/// The scale, in percent, of request `step` where Windows allows `min`
/// steps from the one it recommends.
pub fn step_scale(step: i32, min: i32) -> Option<u32> {
    SCALES.get(usize::try_from(step - min).ok()?).copied()
}

/// Right of the whole desktop, top-aligned with the primary monitor at 0,0, as
/// the macOS helper places its virtual display, so no other monitor moves.
pub fn right_of(others: &[Rect]) -> (i32, i32) {
    (others.iter().map(Rect::right).max().unwrap_or(0), 0)
}

/// Where a monitor at `old` goes when it becomes `size`: its edges that touch
/// another monitor stay put, so a monitor left of or above the others still
/// meets them; otherwise its top-left corner stays.
pub fn resized_origin(old: Rect, size: (u32, u32), others: &[Rect]) -> (i32, i32) {
    let before_right = others
        .iter()
        .any(|other| other.x == old.right() && other.overlaps_rows(&old));
    let before_below = others
        .iter()
        .any(|other| other.y == old.bottom() && other.overlaps_columns(&old));
    (
        if before_right {
            old.right() - size.0 as i32
        } else {
            old.x
        },
        if before_below {
            old.bottom() - size.1 as i32
        } else {
            old.y
        },
    )
}

/// Where a monitor goes at `size`: from `old`, where it is or was, keeping
/// the edges that touch other monitors, as long as it meets one and covers
/// none there; otherwise right of the desktop.
pub fn origin(old: Option<Rect>, size: (u32, u32), others: &[Rect]) -> (i32, i32) {
    old.map(|old| resized_origin(old, size, others))
        .filter(|&(x, y)| {
            let rect = Rect {
                x,
                y,
                width: size.0,
                height: size.1,
            };
            others.is_empty()
                || (others.iter().any(|other| rect.touches(other))
                    && !others.iter().any(|other| rect.overlaps(other)))
        })
        .unwrap_or_else(|| right_of(others))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn presets_and_serials_are_read_leniently() {
        assert_eq!(
            presets(Some("1920x1080, 1366x1024,bad,x5,2388x1668")),
            [(1920, 1080), (1366, 1024), (2388, 1668)]
        );
        assert!(presets(None).is_empty() && presets(Some("")).is_empty());
        let many = ["800x600"; 9].join(",") + ",1024x768";
        assert_eq!(presets(Some(&many)).len(), 8);
        assert_eq!(serial(Some("305419896")), 305_419_896);
        for missing in [None, Some(""), Some("0"), Some("-1"), Some("4294967296")] {
            assert_eq!(serial(missing), DEFAULT_SERIAL, "{missing:?}");
        }
    }

    #[test]
    fn only_a_faster_frame_rate_changes_the_refresh_rate() {
        assert_eq!(refresh_rate(60, 120), 120);
        assert_eq!(refresh_rate(60, 60), 60);
        assert_eq!(refresh_rate(60, 30), 60);
        assert_eq!(refresh_rate(120, 60), 120);
        assert_eq!(refresh_rate(30, 60), 60);
    }

    #[test]
    fn modes_put_the_preferred_mode_and_stream_size_first_and_scale_presets() {
        let presets = [
            (1920, 1080),
            (1194, 834),
            (600, 800),
            (2560, 1600),
            (683, 512),
        ];
        assert_eq!(
            modes((1920, 1080), 1, &presets),
            [(1920, 1080), (1194, 834), (2560, 1600), (683 & !1, 512)]
        );
        // Retina doubles presets in points and drops ones past 4096 pixels.
        assert_eq!(
            modes((2388, 1668), 2, &presets),
            [PREFERRED, (2388, 1668), (3840, 2160), (1366, 1024)]
        );
        assert_eq!(modes((1366, 1024), 1, &[]), [PREFERRED, (1366, 1024)]);
        // The preferred mode and eight more, the stream size among them.
        let many: Vec<_> = (0..8).map(|n| (1024 + n * 16, 768)).collect();
        let offered = modes((1366, 1024), 1, &many);
        assert_eq!(offered.len(), 9);
        assert_eq!(offered[..2], [PREFERRED, (1366, 1024)]);
    }

    #[test]
    fn scales_count_in_steps_from_the_recommended_one() {
        // Windows recommends 125% (one step above 100%) and allows 225%.
        assert_eq!(scale_step(100, -1, 4), Some(-1));
        assert_eq!(scale_step(200, -1, 4), Some(3));
        // Recommended 100%, at most 175%: 200% is out of reach.
        assert_eq!(scale_step(200, 0, 3), Some(3));
        assert_eq!(scale_step(100, 0, 3), Some(0));
        assert_eq!(scale_step(110, 0, 3), None);
        // What a step gives: there, 175% at most.
        assert_eq!(step_scale(3, 0), Some(175));
        assert_eq!(step_scale(3, -1), Some(200));
        assert_eq!(step_scale(-1, -1), Some(100));
        assert_eq!(step_scale(-2, -1), None);
    }

    #[test]
    fn physical_size_is_96_points_per_inch() {
        assert_eq!(millimetres(1920), 508);
        assert_eq!(millimetres(1194), 316);
        assert_eq!(millimetres(96), 25);
    }

    #[test]
    fn a_new_monitor_goes_right_of_the_whole_desktop() {
        assert_eq!(right_of(&[PRIMARY]), (1920, 0));
        let left = Rect {
            x: -1280,
            y: -200,
            width: 1280,
            height: 1024,
        };
        let right = Rect {
            x: 1920,
            y: 300,
            width: 2560,
            height: 1440,
        };
        assert_eq!(right_of(&[left, PRIMARY, right]), (4480, 0));
    }

    #[test]
    fn a_resized_monitor_keeps_touching_its_neighbours() {
        let right = Rect {
            x: 1920,
            y: 0,
            width: 1366,
            height: 1024,
        };
        assert_eq!(resized_origin(right, (2388, 1668), &[PRIMARY]), (1920, 0));
        let left = Rect {
            x: -1366,
            y: 100,
            width: 1366,
            height: 1024,
        };
        assert_eq!(resized_origin(left, (2388, 1668), &[PRIMARY]), (-2388, 100));
        let above = Rect {
            x: 200,
            y: -1024,
            width: 1366,
            height: 1024,
        };
        assert_eq!(
            resized_origin(above, (1920, 1200), &[PRIMARY]),
            (200, -1200)
        );
        // Touching only at a corner is not adjacency.
        let corner = Rect {
            x: -1366,
            y: -1024,
            width: 1366,
            height: 1024,
        };
        assert_eq!(
            resized_origin(corner, (800, 600), &[PRIMARY]),
            (-1366, -1024)
        );
    }

    #[test]
    fn a_monitor_goes_back_where_it_was_if_that_still_fits() {
        let was = Rect {
            x: -1366,
            y: 0,
            width: 1366,
            height: 1024,
        };
        assert_eq!(origin(Some(was), (1366, 1024), &[PRIMARY]), (-1366, 0));
        // Resized, it keeps touching the primary.
        assert_eq!(origin(Some(was), (1920, 1080), &[PRIMARY]), (-1920, 0));
        assert_eq!(origin(None, (1366, 1024), &[PRIMARY]), (1920, 0));
        // Where another monitor went meanwhile, or away from every other
        // one, it goes right of the desktop.
        let taken = Rect {
            x: -1366,
            ..PRIMARY
        };
        assert_eq!(
            origin(Some(was), (1366, 1024), &[PRIMARY, taken]),
            (1920, 0)
        );
        let apart = Rect { x: 4000, ..was };
        assert_eq!(origin(Some(apart), (1366, 1024), &[PRIMARY]), (1920, 0));
        // Beside another extended display that is still there.
        let second = Rect { x: 3286, ..was };
        let first = Rect { x: 1920, ..was };
        assert_eq!(
            origin(Some(second), (1366, 1024), &[PRIMARY, first]),
            (3286, 0)
        );
    }

    #[test]
    fn only_shared_pixels_are_an_overlap() {
        let clone = Rect {
            width: 1366,
            height: 1024,
            ..PRIMARY
        };
        assert!(clone.overlaps(&PRIMARY) && PRIMARY.overlaps(&clone));
        let beside = Rect { x: 1920, ..clone };
        let below = Rect { y: 1080, ..clone };
        let corner = Rect {
            x: 1920,
            y: 1080,
            ..clone
        };
        for rect in [beside, below, corner] {
            assert!(!rect.overlaps(&PRIMARY), "{rect:?}");
        }
        assert!(Rect { x: 1919, ..beside }.overlaps(&PRIMARY));
    }
}
