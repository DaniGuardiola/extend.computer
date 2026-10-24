//! Optional numeric cursor timing; never records pointer coordinates or images.
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct SnapshotThrottle {
    last: Option<std::time::Instant>,
}
#[cfg(any(target_os = "macos", test))]
impl SnapshotThrottle {
    fn admit(&mut self, now: std::time::Instant, enabled: bool) -> bool {
        if !enabled
            || self.last.is_some_and(|last| {
                now.checked_duration_since(last)
                    .is_none_or(|age| age < std::time::Duration::from_secs(1))
            })
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}
#[cfg(target_os = "macos")]
mod native {
    use std::{
        fs::{File, OpenOptions},
        io::Write,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::Path,
        sync::{Mutex, OnceLock},
    };

    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
        fn getuid() -> u32;
    }
    pub(super) fn open(path: &Path) -> Option<File> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(0x100)
            .open(path)
            .ok()?; // Darwin O_NOFOLLOW.
        let info = file.metadata().ok()?;
        if !info.is_file()
            || info.uid() != unsafe { getuid() }
            || info.nlink() != 1
            || info.mode() & 0o777 != 0o600
        {
            return None;
        }
        Some(file)
    }
    fn trace_file() -> Option<&'static Mutex<File>> {
        static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
        FILE.get_or_init(|| {
            std::env::var_os("EXTEND_DISPLAY_METRICS")
                .and_then(|path| open(Path::new(&path)))
                .map(Mutex::new)
        })
        .as_ref()
    }
    fn host_ns() -> Option<u64> {
        let mut info = Timebase { numer: 0, denom: 0 };
        if unsafe { mach_timebase_info(&mut info) } != 0 || info.denom == 0 {
            return None;
        }
        // Same host-time epoch as the native helper's CMClockGetHostTimeClock.
        let at_ns = (unsafe { mach_absolute_time() } as u128 * info.numer as u128
            / info.denom as u128) as u64;
        Some(at_ns)
    }
    pub(super) fn event(stage: &str, sequence: u64, bytes: usize) {
        let Some(file) = trace_file() else {
            return;
        };
        let Some(at_ns) = host_ns() else {
            return;
        };
        let wall_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|v| v.as_nanos())
            .unwrap_or(0);
        let row = format!("{{\"stage\":\"{stage}\",\"at_ns\":{at_ns},\"wall_ns\":{wall_ns},\"frame_ns\":{sequence},\"bytes\":{bytes}}}\n");
        // One append syscall keeps each row intact alongside the helper's writer.
        let _ = file.lock().unwrap().write(row.as_bytes());
    }

    fn snapshot_row(at_ns: u64, snapshot: &serde_json::Value) -> Option<Vec<u8>> {
        let mut row = serde_json::to_vec(&serde_json::json!({"stage":"usb_queue_snapshot", "clock":"mac-host-time-ns", "at_ns":at_ns, "queue_snapshot":snapshot})).ok()?;
        if row.len() > 65_536 {
            return None;
        }
        row.push(b'\n');
        Some(row)
    }
    fn queue_snapshot_to(
        file: Option<&Mutex<File>>,
        throttle: &Mutex<super::SnapshotThrottle>,
        snapshot: &serde_json::Value,
    ) {
        // Missing/invalid cached path returns before clocks, serialization or locks.
        let Some(file) = file else {
            return;
        };
        let mut throttle = throttle.lock().unwrap();
        if !throttle.admit(std::time::Instant::now(), true) {
            return;
        }
        let Some(at_ns) = host_ns() else {
            return;
        };
        let Some(row) = snapshot_row(at_ns, snapshot) else {
            return;
        };
        // One bounded append, same secured file/lock as existing numeric stages.
        let _ = file.lock().unwrap().write(&row);
    }
    pub(super) fn queue_snapshot(snapshot: &serde_json::Value) {
        static THROTTLE: Mutex<super::SnapshotThrottle> =
            Mutex::new(super::SnapshotThrottle { last: None });
        queue_snapshot_to(trace_file(), &THROTTLE, snapshot);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{symlink, PermissionsExt};
        #[test]
        fn actual_snapshot_writer_throttles_bursts_and_disabled_path_leaves_state_untouched() {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("trace");
            let file = Mutex::new(open(&path).unwrap());
            let throttle = Mutex::new(super::super::SnapshotThrottle::default());
            let snapshot = serde_json::json!({"depth":1,"oldest_head_ms":2.0});
            queue_snapshot_to(None, &throttle, &snapshot);
            assert!(throttle.lock().unwrap().last.is_none());
            assert!(std::fs::read(&path).unwrap().is_empty());
            queue_snapshot_to(Some(&file), &throttle, &snapshot);
            queue_snapshot_to(Some(&file), &throttle, &snapshot);
            assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
            let first = throttle.lock().unwrap().last;
            queue_snapshot_to(None, &throttle, &snapshot);
            assert_eq!(throttle.lock().unwrap().last, first);
        }
        #[test]
        fn queue_snapshot_rows_keep_raw_numeric_object_and_secure_file() {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("trace");
            let mut file = open(&path).unwrap();
            let snapshot = serde_json::json!({"carrier":{"outgoing":{"current":{"depth":2,"highwater":8,"oldest_head_ms":12.5}}},"clock":"process_monotonic_local_only"});
            let row = snapshot_row(123, &snapshot).unwrap();
            file.write_all(&row).unwrap();
            let actual: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(actual["queue_snapshot"], snapshot);
            assert_eq!(actual["clock"], "mac-host-time-ns");
            assert_eq!(actual["at_ns"], 123);
            assert!(actual.get("peer").is_none());
            assert!(actual.get("frame_ns").is_none());
            assert!(
                snapshot_row(1, &serde_json::json!({"oversized":"x".repeat(65_536)})).is_none()
            );
        }
        #[test]
        fn trace_rejects_symlink_hardlink_and_public_permissions() {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("trace");
            assert!(open(&path).is_some());
            let link = root.path().join("link");
            symlink(&path, &link).unwrap();
            assert!(open(&link).is_none());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(open(&path).is_none());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::hard_link(&path, root.path().join("hard")).unwrap();
            assert!(open(&path).is_none());
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn cursor_identity(bytes: &[u8]) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let key = if value.get("png").is_some() {
        "revision"
    } else {
        "sequence"
    };
    value[key].as_u64()
}

pub fn cursor_event(stage: &str, bytes: &[u8]) {
    #[cfg(target_os = "macos")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ENABLED.get_or_init(|| std::env::var_os("EXTEND_DISPLAY_METRICS").is_some()) {
            return;
        }
        if let Some(identity) = cursor_identity(bytes) {
            native::event(stage, identity, 0);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (stage, bytes);
}

#[cfg(test)]
mod cursor_identity_tests {
    use super::cursor_identity;

    #[test]
    fn shape_revision_and_position_sequence_join_their_own_stage_chains() {
        assert_eq!(
            cursor_identity(br#"{"png":"image","revision":7,"sequence":99}"#),
            Some(7)
        );
        assert_eq!(
            cursor_identity(br#"{"revision":7,"sequence":99,"x":20,"y":30}"#),
            Some(99)
        );
        assert_eq!(cursor_identity(br#"{"png":"image","revision":0}"#), Some(0));
        assert_eq!(cursor_identity(br#"{"png":"image","sequence":99}"#), None);
        assert_eq!(cursor_identity(br#"{"revision":7}"#), None);
        assert_eq!(cursor_identity(b"invalid"), None);
    }
}

/// The actual configured deadline, not a launcher preference or latency claim.
pub fn reordering_configuration(milliseconds: usize) {
    #[cfg(target_os = "macos")]
    native::event("config_video_reordering_ms", 0, milliseconds);
    #[cfg(not(target_os = "macos"))]
    let _ = milliseconds;
}

/// Experimental receive-clock receipt; "active" means the socket option was
/// verified. Packet counters separately prove timestamps actually used by RTC.
#[cfg(target_os = "macos")]
pub(super) fn kernel_arrival_configuration(active: bool, automatic: bool) {
    native::event("config_kernel_arrival_requested", 0, 1);
    native::event("config_kernel_arrival_active", 0, usize::from(active));
    native::event("config_kernel_arrival_auto", 0, usize::from(automatic));
}

#[cfg(target_os = "macos")]
pub(super) fn kernel_arrival_counts(applied: u64, fallback: u64) {
    native::event(
        "kernel_arrival_applied_packets",
        0,
        applied.try_into().unwrap_or(usize::MAX),
    );
    native::event(
        "kernel_arrival_fallback_packets",
        0,
        fallback.try_into().unwrap_or(usize::MAX),
    );
}

/// Follow str0m's UDP demultiplexer; record DTLS datagram times and lengths only.
/// These include data-channel control/acknowledgments, not just cursor messages.
pub fn datagram_event(stage: &str, bytes: &[u8]) {
    #[cfg(target_os = "macos")]
    if bytes.first().is_some_and(|value| (20..64).contains(value)) {
        native::event(stage, 0, bytes.len());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (stage, bytes);
}

/// Observe existing native USB status polls; no polling thread or packet trace.
/// Snapshot timestamps use the Mac host clock; contained queue ages are local
/// residence durations and must never be subtracted from another device's clock.
pub fn usb_queue_snapshot(snapshot: &serde_json::Value) {
    #[cfg(target_os = "macos")]
    native::queue_snapshot(snapshot);
    #[cfg(not(target_os = "macos"))]
    let _ = snapshot;
}
#[cfg(test)]
mod snapshot_throttle_tests {
    use super::*;
    #[test]
    fn disabled_does_not_consume_budget_and_bursts_are_limited_to_one_hertz() {
        let mut gate = SnapshotThrottle::default();
        let now = std::time::Instant::now();
        assert!(!gate.admit(now, false));
        assert!(gate.last.is_none());
        assert!(gate.admit(now, true));
        for ms in [0, 1, 500, 999] {
            assert!(!gate.admit(now + std::time::Duration::from_millis(ms), true));
        }
        assert!(!gate.admit(now - std::time::Duration::from_millis(1), true));
        assert!(gate.admit(now + std::time::Duration::from_secs(1), true));
    }
}
