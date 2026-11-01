//! The desktop's display topology through the CCD API (`QueryDisplayConfig`,
//! `SetDisplayConfig`). Changes touch only the extended display's own path:
//! every other active path and mode is passed back as it was, and a monitor
//! that Windows switched off or duplicated as ours arrived or left comes
//! back. Only when Windows refuses that does another monitor's duplicate
//! across adapters leave the desktop.
use super::plan::{self, Rect};
use anyhow::{bail, ensure, Context, Result};
use std::{collections::HashMap, mem::size_of};
use windows::Win32::{
    Devices::Display::{
        DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes,
        QueryDisplayConfig, SetDisplayConfig, DISPLAYCONFIG_2DREGION,
        DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_DEVICE_INFO_TYPE, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_0,
        DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_MODE_INFO_TYPE_TARGET,
        DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_PIXELFORMAT_32BPP, DISPLAYCONFIG_RATIONAL,
        DISPLAYCONFIG_SCALING_IDENTITY, DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE,
        DISPLAYCONFIG_SOURCE_MODE, DISPLAYCONFIG_TARGET_DEVICE_NAME, DISPLAYCONFIG_TARGET_MODE,
        DISPLAYCONFIG_VIDEO_SIGNAL_INFO, DISPLAYCONFIG_VIDEO_SIGNAL_INFO_0, QDC_ALL_PATHS,
        QDC_ONLY_ACTIVE_PATHS, QUERY_DISPLAY_CONFIG_FLAGS, SDC_ALLOW_CHANGES,
        SDC_ALLOW_PATH_ORDER_CHANGES, SDC_APPLY, SDC_SAVE_TO_DATABASE,
        SDC_USE_SUPPLIED_DISPLAY_CONFIG, SET_DISPLAY_CONFIG_FLAGS,
    },
    Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID, POINTL},
    Graphics::Gdi::{DISPLAYCONFIG_PATH_ACTIVE, DISPLAYCONFIG_PATH_MODE_IDX_INVALID},
};

/// An adapter and a source or target id on it.
type Key = (i64, u32);

fn luid_key(luid: LUID) -> i64 {
    (i64::from(luid.HighPart) << 32) | i64::from(luid.LowPart)
}

fn source_key(path: &DISPLAYCONFIG_PATH_INFO) -> Key {
    (luid_key(path.sourceInfo.adapterId), path.sourceInfo.id)
}

fn target_key(path: &DISPLAYCONFIG_PATH_INFO) -> Key {
    (luid_key(path.targetInfo.adapterId), path.targetInfo.id)
}

/// A monitor's target: its adapter and its id there, as the driver reports.
#[derive(Debug, Clone)]
pub struct Target {
    pub adapter: LUID,
    pub id: u32,
    /// The monitor's device path, once Windows has set the monitor up. When
    /// Windows duplicates the monitor across adapters, it drives it through
    /// a target of the other adapter's, which only this ties to ours.
    pub monitor: Option<String>,
    /// The refresh rate of every mode the monitor offers, in Hz.
    pub refresh: u32,
}

impl Target {
    fn key(&self) -> Key {
        (luid_key(self.adapter), self.id)
    }
}

/// The device path of the monitor on a target, if it has one yet.
pub fn monitor_path(adapter: LUID, id: u32) -> Option<String> {
    let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: adapter,
            id,
        },
        ..Default::default()
    };
    if unsafe { DisplayConfigGetDeviceInfo(std::ptr::addr_of_mut!(name).cast()) } != 0 {
        return None;
    }
    let path = &name.monitorDevicePath;
    let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    (end > 0).then(|| String::from_utf16_lossy(&path[..end]))
}

#[derive(Clone, Default)]
pub struct Config {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    /// The monitor device path of each active path's target.
    monitors: HashMap<Key, String>,
}

impl Config {
    pub fn query(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<Self> {
        loop {
            let (mut paths, mut modes) = (0u32, 0u32);
            let status = unsafe { GetDisplayConfigBufferSizes(flags, &mut paths, &mut modes) };
            ensure!(
                status == ERROR_SUCCESS,
                "Could not read the display layout ({})",
                status.0
            );
            let mut config = Self {
                paths: vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize],
                modes: vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize],
                monitors: HashMap::new(),
            };
            let status = unsafe {
                QueryDisplayConfig(
                    flags,
                    &mut paths,
                    config.paths.as_mut_ptr(),
                    &mut modes,
                    config.modes.as_mut_ptr(),
                    None,
                )
            };
            // The layout changed between the two calls.
            if status == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            ensure!(
                status == ERROR_SUCCESS,
                "Could not read the display layout ({})",
                status.0
            );
            config.paths.truncate(paths as usize);
            config.modes.truncate(modes as usize);
            config.monitors = config
                .active_paths()
                .filter_map(|path| {
                    let monitor = monitor_path(path.targetInfo.adapterId, path.targetInfo.id)?;
                    Some((target_key(path), monitor))
                })
                .collect();
            return Ok(config);
        }
    }

    pub fn active() -> Result<Self> {
        Self::query(QDC_ONLY_ACTIVE_PATHS)
    }

    fn active_paths(&self) -> impl Iterator<Item = &DISPLAYCONFIG_PATH_INFO> {
        self.paths
            .iter()
            .filter(|path| path.flags & DISPLAYCONFIG_PATH_ACTIVE != 0)
    }

    /// Whether `path` shows `target`'s monitor, on its own target or on
    /// another adapter's.
    fn is(&self, target: &Target, path: &DISPLAYCONFIG_PATH_INFO) -> bool {
        target_key(path) == target.key()
            || target
                .monitor
                .as_ref()
                .is_some_and(|monitor| self.monitors.get(&target_key(path)) == Some(monitor))
    }

    /// The desktop rectangle of a path's source.
    fn source_rect(&self, path: &DISPLAYCONFIG_PATH_INFO) -> Option<Rect> {
        let mode = self
            .modes
            .get(unsafe { path.sourceInfo.Anonymous.modeInfoIdx } as usize)?;
        (mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE).then(|| {
            let source = unsafe { mode.Anonymous.sourceMode };
            Rect {
                x: source.position.x,
                y: source.position.y,
                width: source.width,
                height: source.height,
            }
        })
    }

    /// Where `target` is on the desktop, and whether it duplicates another
    /// monitor rather than extending the desktop: it shows that monitor's
    /// source, through another adapter's target or not, or a source at the
    /// same place.
    pub fn find(&self, target: &Target) -> Option<(Rect, bool)> {
        let path = self
            .active_paths()
            .filter(|path| self.is(target, path))
            .min_by_key(|path| target_key(path) != target.key())?;
        let rect = self.source_rect(path)?;
        let shared = target_key(path) != target.key()
            || self
                .active_paths()
                .filter(|other| !self.is(target, other))
                .any(|other| {
                    source_key(other) == source_key(path)
                        || self.source_rect(other).is_some_and(|r| r.overlaps(&rect))
                });
        Some((rect, shared))
    }

    /// The size of the signal Windows sends `target`'s own path, which is
    /// the size of the frames the driver gets.
    pub fn signal(&self, target: &Target) -> Option<(u32, u32)> {
        let path = self
            .active_paths()
            .find(|path| target_key(path) == target.key())?;
        let mode = self
            .modes
            .get(unsafe { path.targetInfo.Anonymous.modeInfoIdx } as usize)?;
        (mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET).then(|| {
            let info = unsafe { mode.Anonymous.targetMode.targetVideoSignalInfo };
            (info.activeSize.cx, info.activeSize.cy)
        })
    }

    /// The desktop rectangles of the active paths' sources.
    fn rects(&self) -> Vec<Rect> {
        let mut seen = Vec::new();
        self.active_paths()
            .filter(|path| {
                let key = source_key(path);
                let new = !seen.contains(&key);
                seen.push(key);
                new
            })
            .filter_map(|path| self.source_rect(path))
            .collect()
    }

    /// Adds `path` of `from` with the modes it uses. `remap` maps `from`'s
    /// mode indices to this configuration's, so modes that several paths
    /// share stay shared.
    fn push(
        &mut self,
        from: &Config,
        path: &DISPLAYCONFIG_PATH_INFO,
        remap: &mut HashMap<u32, u32>,
    ) {
        let mut keep = |index: u32| -> u32 {
            if index == DISPLAYCONFIG_PATH_MODE_IDX_INVALID || index as usize >= from.modes.len() {
                return DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
            }
            *remap.entry(index).or_insert_with(|| {
                self.modes.push(from.modes[index as usize]);
                self.modes.len() as u32 - 1
            })
        };
        let mut path = *path;
        unsafe {
            path.sourceInfo.Anonymous.modeInfoIdx = keep(path.sourceInfo.Anonymous.modeInfoIdx);
            path.targetInfo.Anonymous.modeInfoIdx = keep(path.targetInfo.Anonymous.modeInfoIdx);
        }
        if let Some(monitor) = from.monitors.get(&target_key(&path)) {
            self.monitors.insert(target_key(&path), monitor.clone());
        }
        self.paths.push(path);
    }

    /// This configuration without the paths to `targets`.
    fn without(&self, targets: &[Key]) -> Config {
        let mut config = Config::default();
        let mut remap = HashMap::new();
        for path in self
            .paths
            .iter()
            .filter(|path| !targets.contains(&target_key(path)))
        {
            config.push(self, path, &mut remap);
        }
        config
    }

    fn set(&self, flags: SET_DISPLAY_CONFIG_FLAGS) -> i32 {
        unsafe { SetDisplayConfig(Some(&self.paths), Some(&self.modes), flags) }
    }
}

/// The targets of `config`'s paths that duplicate another monitor across
/// adapters, the way Windows duplicates a new monitor: a path that shares
/// its source with another one and drives a monitor whose own target is on
/// another adapter of `all`, which `name` (the device path of a target's
/// monitor) ties to it.
fn duplicates(
    config: &Config,
    all: &Config,
    name: impl Fn(LUID, u32) -> Option<String>,
) -> Vec<Key> {
    let mut names: HashMap<Key, Option<String>> = HashMap::new();
    let mut duplicates = Vec::new();
    for path in &config.paths {
        let key = target_key(path);
        let Some(monitor) = config.monitors.get(&key) else {
            continue;
        };
        let cloned = config
            .paths
            .iter()
            .any(|other| target_key(other) != key && source_key(other) == source_key(path));
        let elsewhere = cloned
            && all.paths.iter().any(|other| {
                let other_key = target_key(other);
                other_key.0 != key.0
                    && names
                        .entry(other_key)
                        .or_insert_with(|| name(other.targetInfo.adapterId, other.targetInfo.id))
                        .as_ref()
                        == Some(monitor)
            });
        if elsewhere {
            duplicates.push(key);
        }
    }
    duplicates
}

/// The paths of every monitor but `target`'s: those active in `current`,
/// and those of `earlier` that Windows has switched off since, or made a
/// duplicate of another monitor through another adapter's target, while
/// their monitor is still connected (`all`) and their source is free. A
/// monitor that comes back loses its duplicate's path. Returns how many came
/// back from `earlier`.
fn kept(
    current: &Config,
    earlier: Option<&Config>,
    all: &Config,
    target: &Target,
) -> (Config, usize) {
    let others: Vec<_> = current
        .active_paths()
        .filter(|path| !current.is(target, path))
        .collect();
    let none = Config::default();
    let earlier = earlier.unwrap_or(&none);
    let mut back: Vec<&DISPLAYCONFIG_PATH_INFO> = Vec::new();
    let mut dropped: Vec<Key> = Vec::new();
    for path in earlier.active_paths() {
        let key = target_key(path);
        if earlier.is(target, path) || others.iter().any(|other| target_key(other) == key) {
            continue;
        }
        // The current paths that show its monitor through another target.
        let monitor = earlier.monitors.get(&key);
        let duplicates: Vec<Key> = others
            .iter()
            .map(|other| target_key(other))
            .filter(|other| monitor.is_some() && current.monitors.get(other) == monitor)
            .collect();
        let connected = !duplicates.is_empty()
            || all.paths.iter().any(|candidate| {
                target_key(candidate) == key && candidate.targetInfo.targetAvailable.as_bool()
            });
        let taken = others.iter().any(|other| {
            source_key(other) == source_key(path) && !duplicates.contains(&target_key(other))
        }) || back
            .iter()
            .any(|other| source_key(other) == source_key(path));
        if connected && !taken {
            dropped.extend(duplicates);
            back.push(path);
        }
    }
    let mut config = Config::default();
    let mut remap = HashMap::new();
    for path in others
        .iter()
        .filter(|other| !dropped.contains(&target_key(other)))
    {
        config.push(current, path, &mut remap);
    }
    let mut remap = HashMap::new();
    for path in &back {
        config.push(earlier, path, &mut remap);
    }
    (config, back.len())
}

/// `kept` with `target` its own part of the desktop at `origin` and `size`.
/// The target keeps the source it shows if no other monitor shares it, or
/// takes a free one. With `exact`, Windows sends the monitor a signal of
/// that size too; otherwise it picks one, which may be the preferred mode's
/// with the desktop scaled onto it.
fn extended(
    kept: &Config,
    current: &Config,
    all: &Config,
    target: &Target,
    size: (u32, u32),
    origin: (i32, i32),
    exact: bool,
) -> Result<Config> {
    let mut config = kept.clone();
    let used: Vec<Key> = config.paths.iter().map(source_key).collect();
    let now = current
        .active_paths()
        .find(|path| target_key(path) == target.key())
        .map(source_key);
    // Windows reports our target unavailable while it drives the monitor
    // through another adapter's; leaving that path out frees it.
    let mut candidates: Vec<_> = all
        .paths
        .iter()
        .filter(|path| target_key(path) == target.key() && !used.contains(&source_key(path)))
        .collect();
    candidates.sort_by_key(|path| Some(source_key(path)) != now);
    let Some(&&candidate) = candidates.first() else {
        bail!("Windows offers no free display source for the extended display");
    };
    let mut path = candidate;
    path.flags = DISPLAYCONFIG_PATH_ACTIVE;
    config.modes.push(source_mode(
        path.sourceInfo.adapterId,
        path.sourceInfo.id,
        size,
        origin,
    ));
    path.sourceInfo.Anonymous.modeInfoIdx = config.modes.len() as u32 - 1;
    path.targetInfo.Anonymous.modeInfoIdx = if exact {
        config.modes.push(target_mode(target, size));
        path.targetInfo.refreshRate = DISPLAYCONFIG_RATIONAL {
            Numerator: target.refresh,
            Denominator: 1,
        };
        path.targetInfo.scaling = DISPLAYCONFIG_SCALING_IDENTITY;
        config.modes.len() as u32 - 1
    } else {
        DISPLAYCONFIG_PATH_MODE_IDX_INVALID
    };
    if let Some(monitor) = &target.monitor {
        config.monitors.insert(target.key(), monitor.clone());
    }
    config.paths.push(path);
    Ok(config)
}

/// The layout around `target` as it is now.
pub struct Layout {
    current: Config,
    all: Config,
    kept: Config,
    restored: usize,
}

impl Layout {
    /// Reads the layout; monitors of `earlier` that Windows switched off
    /// since come back with the next change.
    pub fn read(target: &Target, earlier: Option<&Config>) -> Result<Self> {
        let current = Config::active()?;
        let all = Config::query(QDC_ALL_PATHS)?;
        let (kept, restored) = kept(&current, earlier, &all, target);
        Ok(Self {
            current,
            all,
            kept,
            restored,
        })
    }

    /// The size of the signal Windows sends `target` now.
    pub fn signal(&self, target: &Target) -> Option<(u32, u32)> {
        self.current.signal(target)
    }

    /// Whether monitors come back.
    pub fn restores(&self) -> bool {
        self.restored > 0
    }

    /// The desktop rectangles of the other monitors.
    pub fn others(&self) -> Vec<Rect> {
        self.kept.rects()
    }

    /// Applies the layout with `target` at `origin` and `size`, and saves it,
    /// so Windows extends the same way when these monitors next connect.
    /// Without the monitors that came back if Windows refuses them, and
    /// failing that without another monitor's duplicate across adapters,
    /// which Windows refuses to take back as it is (that monitor leaves the
    /// desktop). Returns how many duplicates were left out.
    pub fn extend(&self, target: &Target, size: (u32, u32), origin: (i32, i32)) -> Result<usize> {
        let mut status = self.apply(&self.kept, target, size, origin)?;
        if status != 0 && self.restored > 0 {
            let others = kept(&self.current, None, &self.all, target).0;
            status = self.apply(&others, target, size, origin)?;
        }
        if status == 0 {
            return Ok(0);
        }
        let duplicates = duplicates(&self.kept, &self.all, monitor_path);
        if !duplicates.is_empty() {
            status = self.apply(&self.kept.without(&duplicates), target, size, origin)?;
            if status == 0 {
                return Ok(duplicates.len());
            }
        }
        bail!("Windows did not accept the extended display layout ({status})")
    }

    /// Tries to apply `others` with `target` at `origin` and `size`, first
    /// with the driver's timing for it, then letting Windows pick. Returns
    /// the last `SetDisplayConfig` status: 0 once one was applied.
    fn apply(
        &self,
        others: &Config,
        target: &Target,
        size: (u32, u32),
        origin: (i32, i32),
    ) -> Result<i32> {
        let flags = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE;
        let attempts = [
            // As given: allowed to change it, Windows would send the
            // monitor its preferred mode's signal and scale the desktop.
            (true, flags),
            (true, flags | SDC_ALLOW_CHANGES),
            (false, flags | SDC_ALLOW_CHANGES),
            (
                false,
                flags | SDC_ALLOW_CHANGES | SDC_ALLOW_PATH_ORDER_CHANGES,
            ),
        ];
        let mut status = 0;
        for (exact, flags) in attempts {
            let config = extended(
                others,
                &self.current,
                &self.all,
                target,
                size,
                origin,
                exact,
            )?;
            status = config.set(flags);
            if status == 0 {
                break;
            }
        }
        Ok(status)
    }

    /// Applies the layout without `target`, bringing back the monitors
    /// Windows switched off; returns the `SetDisplayConfig` status.
    pub fn restore(&self) -> i32 {
        self.kept
            .set(SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES)
    }
}

/// The driver's timing for `target` at `size`: it describes every mode it
/// offers without blanking (`FillSignalInfo` in
/// `native/windows/driver/Driver.cpp`), and Windows takes only a target
/// mode it offers.
fn target_mode(target: &Target, size: (u32, u32)) -> DISPLAYCONFIG_MODE_INFO {
    let (width, height, refresh) = (size.0, size.1, target.refresh);
    let region = DISPLAYCONFIG_2DREGION {
        cx: width,
        cy: height,
    };
    DISPLAYCONFIG_MODE_INFO {
        infoType: DISPLAYCONFIG_MODE_INFO_TYPE_TARGET,
        id: target.id,
        adapterId: target.adapter,
        Anonymous: DISPLAYCONFIG_MODE_INFO_0 {
            targetMode: DISPLAYCONFIG_TARGET_MODE {
                targetVideoSignalInfo: DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
                    pixelRate: u64::from(refresh) * u64::from(width) * u64::from(height),
                    hSyncFreq: DISPLAYCONFIG_RATIONAL {
                        Numerator: refresh * height,
                        Denominator: 1,
                    },
                    vSyncFreq: DISPLAYCONFIG_RATIONAL {
                        Numerator: refresh,
                        Denominator: 1,
                    },
                    activeSize: region,
                    totalSize: region,
                    // D3DKMDT_VSS_OTHER, with a vertical sync divider of 1.
                    Anonymous: DISPLAYCONFIG_VIDEO_SIGNAL_INFO_0 {
                        videoStandard: 255 | 1 << 16,
                    },
                    scanLineOrdering: DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE,
                },
            },
        },
    }
}

fn source_mode(
    adapter: LUID,
    id: u32,
    size: (u32, u32),
    origin: (i32, i32),
) -> DISPLAYCONFIG_MODE_INFO {
    DISPLAYCONFIG_MODE_INFO {
        infoType: DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
        id,
        adapterId: adapter,
        Anonymous: DISPLAYCONFIG_MODE_INFO_0 {
            sourceMode: DISPLAYCONFIG_SOURCE_MODE {
                width: size.0,
                height: size.1,
                pixelFormat: DISPLAYCONFIG_PIXELFORMAT_32BPP,
                position: POINTL {
                    x: origin.0,
                    y: origin.1,
                },
            },
        },
    }
}

// The scale requests Windows' display settings send. Windows doesn't
// document them; they count in its scale steps, relative to the one it
// recommends for the monitor's size and mode.
const GET_DPI_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-3);
const SET_DPI_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-4);

#[repr(C)]
struct DpiScaleGet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min: i32,
    current: i32,
    max: i32,
}

#[repr(C)]
struct DpiScaleSet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    scale: i32,
}

/// What [`set_scale`] did.
pub struct Rescaled {
    pub changed: bool,
    /// The scale, in percent: the one asked for, or the nearest Windows
    /// allows there.
    pub percent: u32,
}

/// Sets the scale of `target`'s part of the desktop to `percent`, or the
/// nearest Windows allows there. Windows recommends a scale from the
/// physical size and the mode, so it would drift as the mode changes.
pub fn set_scale(target: &Target, percent: u32) -> Result<Rescaled> {
    let config = Config::active()?;
    let path = config
        .active_paths()
        .find(|path| target_key(path) == target.key())
        .context("The extended display is not on the desktop")?;
    let header = |kind, size: usize| DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size as u32,
        adapterId: path.sourceInfo.adapterId,
        id: path.sourceInfo.id,
    };
    let mut get = DpiScaleGet {
        header: header(GET_DPI_SCALE, size_of::<DpiScaleGet>()),
        min: 0,
        current: 0,
        max: 0,
    };
    let status = unsafe { DisplayConfigGetDeviceInfo(std::ptr::addr_of_mut!(get).cast()) };
    ensure!(status == 0, "Could not read the display scale ({status})");
    let Some(step) = plan::scale_step(percent, get.min, get.max) else {
        bail!("Windows offers no {percent}% scale here");
    };
    let rescaled = Rescaled {
        changed: step != get.current,
        percent: plan::step_scale(step, get.min).unwrap_or(percent),
    };
    if !rescaled.changed {
        return Ok(rescaled);
    }
    let set = DpiScaleSet {
        header: header(SET_DPI_SCALE, size_of::<DpiScaleSet>()),
        scale: step,
    };
    let status = unsafe { DisplayConfigSetDeviceInfo(std::ptr::addr_of!(set).cast()) };
    ensure!(status == 0, "Could not set the display scale ({status})");
    Ok(rescaled)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPU: LUID = LUID {
        LowPart: 7,
        HighPart: 0,
    };
    const IDD: LUID = LUID {
        LowPart: 9,
        HighPart: 1,
    };
    const PANEL: u32 = 4355;
    /// The target through which the GPU drives our monitor when Windows
    /// clones the panel onto it.
    const ALIAS: u32 = 0x4000_0100;
    const OURS: &str = r"\\?\DISPLAY#XEC0001#1&28a6823a&6&UID256";

    fn ours() -> Target {
        Target {
            adapter: IDD,
            id: 256,
            monitor: Some(OURS.into()),
            refresh: 60,
        }
    }

    fn path(
        source: (LUID, u32),
        target: (LUID, u32),
        modes: (u32, u32),
        active: bool,
    ) -> DISPLAYCONFIG_PATH_INFO {
        let mut path = DISPLAYCONFIG_PATH_INFO::default();
        path.sourceInfo.adapterId = source.0;
        path.sourceInfo.id = source.1;
        path.sourceInfo.Anonymous.modeInfoIdx = modes.0;
        path.targetInfo.adapterId = target.0;
        path.targetInfo.id = target.1;
        path.targetInfo.Anonymous.modeInfoIdx = modes.1;
        path.targetInfo.targetAvailable = true.into();
        path.flags = if active { DISPLAYCONFIG_PATH_ACTIVE } else { 0 };
        path
    }

    fn target_mode(adapter: LUID, id: u32) -> DISPLAYCONFIG_MODE_INFO {
        DISPLAYCONFIG_MODE_INFO {
            infoType: DISPLAYCONFIG_MODE_INFO_TYPE_TARGET,
            id,
            adapterId: adapter,
            ..Default::default()
        }
    }

    const NONE: u32 = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    const PANEL_RECT: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    /// The internal panel at 0,0, our monitor on `ours` (a source and its
    /// rectangle) if given, and every source the IddCx adapter offers our
    /// target.
    fn desktop(ours: Option<(u32, Rect)>) -> (Config, Config) {
        let mut active = Config {
            paths: vec![path((GPU, 0), (GPU, PANEL), (0, 1), true)],
            modes: vec![
                source_mode(GPU, 0, (1920, 1080), (0, 0)),
                target_mode(GPU, PANEL),
            ],
            monitors: HashMap::from([((luid_key(GPU), PANEL), "panel".into())]),
        };
        if let Some((source, rect)) = ours {
            active.modes.push(source_mode(
                IDD,
                source,
                (rect.width, rect.height),
                (rect.x, rect.y),
            ));
            active.modes.push(target_mode(IDD, 256));
            active
                .paths
                .push(path((IDD, source), (IDD, 256), (2, 3), true));
            active.monitors.insert((luid_key(IDD), 256), OURS.into());
        }
        let mut all = active.clone();
        for source in 0..4 {
            all.paths
                .push(path((IDD, source), (IDD, 256), (NONE, NONE), false));
        }
        (active, all)
    }

    fn mode_index(path: &DISPLAYCONFIG_PATH_INFO) -> (u32, u32) {
        unsafe {
            (
                path.sourceInfo.Anonymous.modeInfoIdx,
                path.targetInfo.Anonymous.modeInfoIdx,
            )
        }
    }

    fn place(active: &Config, all: &Config, size: (u32, u32), origin: (i32, i32)) -> Config {
        let (others, _) = kept(active, None, all, &ours());
        extended(&others, active, all, &ours(), size, origin, true).unwrap()
    }

    #[test]
    fn a_new_monitor_gets_a_free_source_and_others_stay() {
        let (active, all) = desktop(None);
        let config = place(&active, &all, (1366, 1024), (1920, 0));
        assert_eq!(config.paths.len(), 2);
        // The panel's path and modes are passed back unchanged.
        assert_eq!(mode_index(&config.paths[0]), (0, 1));
        assert_eq!(config.source_rect(&config.paths[0]), Some(PANEL_RECT));
        let path = &config.paths[1];
        assert!(target_key(path) == ours().key() && path.flags & DISPLAYCONFIG_PATH_ACTIVE != 0);
        assert_eq!(mode_index(path), (2, 3));
        // The signal is the driver's timing for the mode, so Windows doesn't
        // scale the desktop onto the preferred mode's.
        assert_eq!(config.signal(&ours()), Some((1366, 1024)));
        let info = unsafe { config.modes[3].Anonymous.targetMode.targetVideoSignalInfo };
        assert_eq!(
            (
                info.pixelRate,
                info.hSyncFreq.Numerator,
                info.vSyncFreq.Numerator,
                info.totalSize.cx,
                unsafe { info.Anonymous.videoStandard }
            ),
            (60 * 1366 * 1024, 60 * 1024, 60, 1366, 0x100ff)
        );
        assert_eq!(path.targetInfo.scaling, DISPLAYCONFIG_SCALING_IDENTITY);
        // Without it, Windows picks the signal.
        let (others, _) = kept(&active, None, &all, &ours());
        let loose = extended(
            &others,
            &active,
            &all,
            &ours(),
            (1366, 1024),
            (1920, 0),
            false,
        )
        .unwrap();
        assert_eq!(mode_index(&loose.paths[1]), (2, NONE));
        let rect = Rect {
            x: 1920,
            y: 0,
            width: 1366,
            height: 1024,
        };
        assert_eq!(config.source_rect(path), Some(rect));
        assert_eq!(config.find(&ours()), Some((rect, false)));
    }

    #[test]
    fn a_resized_monitor_keeps_its_source() {
        let rect = Rect {
            x: -1366,
            y: 0,
            width: 1366,
            height: 1024,
        };
        let (active, all) = desktop(Some((2, rect)));
        assert_eq!(active.find(&ours()), Some((rect, false)));
        let (others, restored) = kept(&active, None, &all, &ours());
        assert_eq!((others.rects(), restored), (vec![PANEL_RECT], 0));
        let config = place(&active, &all, (2388, 1668), (-2388, 0));
        assert_eq!(config.paths.len(), 2);
        assert_eq!(config.paths[1].sourceInfo.id, 2);
        // Our old modes are dropped rather than left unreferenced: the
        // panel's two and our new source and target modes.
        assert_eq!(config.modes.len(), 4);
        assert_eq!(config.signal(&ours()), Some((2388, 1668)));
    }

    #[test]
    fn a_cross_adapter_clone_is_ours_and_a_duplicate() {
        // What Windows does with a new monitor on this PC: the GPU shows
        // the panel's source on a target of its own that drives our
        // monitor, and our own target's paths turn unavailable.
        let (mut active, mut all) = desktop(None);
        let clone = path((GPU, 0), (GPU, ALIAS), (0, NONE), true);
        active.paths.push(clone);
        active.monitors.insert((luid_key(GPU), ALIAS), OURS.into());
        all.paths.push(clone);
        for path in &mut all.paths {
            if target_key(path) == ours().key() {
                path.targetInfo.targetAvailable = false.into();
            }
        }
        assert_eq!(active.find(&ours()), Some((PANEL_RECT, true)));
        // Without its device path the clone isn't ours.
        let unnamed = Target {
            monitor: None,
            ..ours()
        };
        assert_eq!(active.find(&unnamed), None);
        // The panel is still in the way of where the monitor goes.
        let (others, _) = kept(&active, None, &all, &ours());
        assert_eq!(others.rects(), [PANEL_RECT]);
        let config = place(&active, &all, (1920, 1080), (1920, 0));
        // The clone's path is gone and our own target has a source.
        assert_eq!(config.paths.len(), 2);
        assert_eq!(target_key(&config.paths[1]), ours().key());
        assert_eq!(config.paths[1].sourceInfo.adapterId, IDD);
        assert_eq!(
            config.find(&ours()),
            Some((
                Rect {
                    x: 1920,
                    ..PANEL_RECT
                },
                false
            ))
        );
    }

    #[test]
    fn a_clone_on_its_own_adapter_is_a_duplicate() {
        let (active, all) = desktop(Some((0, PANEL_RECT)));
        assert_eq!(active.find(&ours()), Some((PANEL_RECT, true)));
        let config = place(&active, &all, (1920, 1080), (1920, 0));
        // It keeps its source, now beside the panel.
        assert_eq!(config.paths[1].sourceInfo.id, 0);
        assert_eq!(
            config.find(&ours()),
            Some((
                Rect {
                    x: 1920,
                    ..PANEL_RECT
                },
                false
            ))
        );
    }

    /// The panel with another extended display, `(IDD, 1) -> (IDD, 257)`,
    /// right of it.
    fn with_another() -> Config {
        let (mut earlier, _) = desktop(None);
        earlier
            .modes
            .push(source_mode(IDD, 1, (1366, 1024), (1920, 0)));
        earlier
            .paths
            .push(path((IDD, 1), (IDD, 257), (2, NONE), true));
        earlier
            .monitors
            .insert((luid_key(IDD), 257), "another".into());
        earlier
    }

    #[test]
    fn a_monitor_windows_switched_off_comes_back() {
        let earlier = with_another();
        // Ours arrived and Windows switched the other one off.
        let (active, mut all) = desktop(None);
        all.paths
            .push(path((IDD, 1), (IDD, 257), (NONE, NONE), false));
        let (others, restored) = kept(&active, Some(&earlier), &all, &ours());
        assert_eq!(restored, 1);
        let other = Rect {
            x: 1920,
            y: 0,
            width: 1366,
            height: 1024,
        };
        assert_eq!(others.rects(), [PANEL_RECT, other]);
        assert_eq!(plan::right_of(&others.rects()), (3286, 0));
        let config = extended(
            &others,
            &active,
            &all,
            &ours(),
            (1920, 1080),
            (3286, 0),
            true,
        )
        .unwrap();
        assert_eq!(config.paths.len(), 3);
        assert_eq!(config.source_rect(&config.paths[1]), Some(other));
        // Our source isn't the other monitor's.
        assert_eq!(config.paths[2].sourceInfo.id, 0);
        // Nothing comes back while it shows, or once it is unplugged.
        let (_, restored) = kept(&earlier, Some(&earlier), &all, &ours());
        assert_eq!(restored, 0);
        for path in &mut all.paths {
            if path.targetInfo.id == 257 {
                path.targetInfo.targetAvailable = false.into();
            }
        }
        assert_eq!(kept(&active, Some(&earlier), &all, &ours()).1, 0);
    }

    #[test]
    fn a_monitor_windows_duplicated_comes_back_on_its_own_target() {
        let earlier = with_another();
        // As ours arrived or left, Windows made the other one a duplicate
        // of the panel, through a target of the GPU's.
        let (mut active, mut all) = desktop(None);
        let alias = (GPU, 0x4000_0101);
        active.paths.push(path((GPU, 0), alias, (0, NONE), true));
        active
            .monitors
            .insert((luid_key(alias.0), alias.1), "another".into());
        all.paths
            .push(path((IDD, 1), (IDD, 257), (NONE, NONE), false));
        all.paths.last_mut().unwrap().targetInfo.targetAvailable = false.into();
        let (others, restored) = kept(&active, Some(&earlier), &all, &ours());
        assert_eq!(restored, 1);
        // The duplicate's path is gone and the monitor is back where it was.
        assert_eq!(others.paths.len(), 2);
        assert_eq!(target_key(&others.paths[1]), (luid_key(IDD), 257));
        assert_eq!(
            others.rects(),
            [
                PANEL_RECT,
                Rect {
                    x: 1920,
                    y: 0,
                    width: 1366,
                    height: 1024
                }
            ]
        );
    }

    #[test]
    fn another_monitors_duplicate_across_adapters_can_be_left_out() {
        // Another extended display arrived as ours is placed, and Windows
        // duplicated the panel onto it through a target of the GPU's.
        let (mut active, mut all) = desktop(None);
        let alias = (GPU, 0x4000_0101);
        let clone = path((GPU, 0), alias, (0, NONE), true);
        active.paths.push(clone);
        active
            .monitors
            .insert((luid_key(alias.0), alias.1), "another".into());
        all.paths.push(clone);
        for source in 0..4 {
            let mut own = path((IDD, source), (IDD, 257), (NONE, NONE), false);
            own.targetInfo.targetAvailable = false.into();
            all.paths.push(own);
        }
        let name = |adapter: LUID, id: u32| {
            let key = (luid_key(adapter), id);
            if key == (luid_key(IDD), 257) || key == (luid_key(alias.0), alias.1) {
                Some("another".to_string())
            } else if key == (luid_key(GPU), PANEL) {
                Some("panel".into())
            } else {
                None
            }
        };
        let (others, _) = kept(&active, None, &all, &ours());
        assert_eq!(others.paths.len(), 2);
        let duplicated = duplicates(&others, &all, name);
        assert_eq!(duplicated, [(luid_key(alias.0), alias.1)]);
        // Without it, the panel's path and modes are as they were.
        let without = others.without(&duplicated);
        assert_eq!(without.paths.len(), 1);
        assert_eq!(mode_index(&without.paths[0]), (0, 1));
        assert_eq!(without.rects(), [PANEL_RECT]);
        let config = extended(
            &without,
            &active,
            &all,
            &ours(),
            (1920, 1080),
            (1920, 0),
            true,
        )
        .unwrap();
        assert_eq!(config.paths.len(), 2);
        assert_eq!(target_key(&config.paths[1]), ours().key());
        // A monitor that extends the desktop from a source of its own is
        // no duplicate.
        assert!(duplicates(&with_another(), &all, name).is_empty());
    }

    #[test]
    fn no_free_source_is_an_error() {
        let (active, mut all) = desktop(None);
        all.paths.retain(|path| target_key(path) != ours().key());
        let (others, _) = kept(&active, None, &all, &ours());
        assert!(extended(
            &others,
            &active,
            &all,
            &ours(),
            (1920, 1080),
            (1920, 0),
            true
        )
        .is_err());
    }
}
