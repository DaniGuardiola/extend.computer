//! Opt-in, bounded native-carrier queue diagnostics. No packet payloads or IDs.
use serde_json::{json, Value};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

const BOUNDS_MS: [f64; 12] = [
    0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 250.0, 500.0, 1000.0,
];
pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let env = std::env::var_os("EXTEND_DISPLAY_METRICS").is_some();
        #[cfg(target_os = "android")]
        {
            extern "C" {
                fn __system_property_get(
                    name: *const std::ffi::c_char,
                    value: *mut std::ffi::c_char,
                ) -> i32;
            }
            let mut value: [std::ffi::c_char; 92] = [0; 92];
            let count = unsafe {
                __system_property_get(c"debug.extend.display_metrics".as_ptr(), value.as_mut_ptr())
            };
            env || (count == 1 && value[0] == b'1' as std::ffi::c_char)
        }
        #[cfg(not(target_os = "android"))]
        env
    })
}
#[derive(Clone, Copy)]
pub(super) struct Stamp {
    at: Instant,
    class: usize,
}
pub(super) fn stamp(bytes: &[u8], prefix: usize) -> Option<Stamp> {
    enabled().then(|| Stamp {
        at: Instant::now(),
        class: packet_class(bytes, prefix),
    })
}
fn packet_class(bytes: &[u8], prefix: usize) -> usize {
    let valid = if prefix == 4 {
        bytes.len() > 4
            && u16::from_be_bytes([bytes[0], bytes[1]]) > 40000
            && u16::from_be_bytes([bytes[2], bytes[3]]) > 40000
    } else {
        prefix == 0
    };
    classify(valid.then(|| bytes.get(prefix).copied()).flatten())
}
fn classify(first: Option<u8>) -> usize {
    match first {
        Some(20..=63) => 0,
        Some(128..=191) => 1,
        _ => 2,
    }
}
#[derive(Default)]
struct Samples {
    count: u64,
    max_ms: f64,
    bins: [u64; 13],
}
impl Samples {
    fn record(&mut self, age: Duration) {
        let ms = age.as_secs_f64() * 1000.0;
        self.count += 1;
        self.max_ms = self.max_ms.max(ms);
        self.bins[BOUNDS_MS.iter().position(|b| ms <= *b).unwrap_or(12)] += 1;
    }
    fn json(&self) -> Value {
        let threshold = self.count.saturating_mul(95).div_ceil(100);
        let mut total = 0;
        let p95 = self.bins.iter().enumerate().find_map(|(i, n)| {
            total += n;
            (self.count > 0 && total >= threshold)
                .then(|| BOUNDS_MS.get(i).copied().unwrap_or(self.max_ms))
        });
        json!({"count":self.count,"max_ms":self.max_ms,"p95_bucket_upper_ms":p95,"histogram":self.bins})
    }
}
pub(super) struct Trace {
    label: &'static str,
    window: Instant,
    samples: [Samples; 3],
    highwater: usize,
    last: Value,
}
impl Trace {
    #[cfg(test)]
    pub(super) fn testing(label: &'static str) -> Self {
        Self {
            label,
            window: Instant::now(),
            samples: Default::default(),
            highwater: 0,
            last: Value::Null,
        }
    }
    pub(super) fn new(label: &'static str) -> Option<Self> {
        enabled().then(|| Self {
            label,
            window: Instant::now(),
            samples: Default::default(),
            highwater: 0,
            last: Value::Null,
        })
    }
    pub(super) fn depth(&mut self, depth: usize) {
        self.highwater = self.highwater.max(depth);
    }
    pub(super) fn removed(&mut self, stamp: Option<Stamp>) {
        if let Some(stamp) = stamp {
            self.samples[stamp.class].record(stamp.at.elapsed());
        }
    }
    pub(super) fn snapshot(&mut self, depth: usize, head: Option<Stamp>) -> Value {
        self.depth(depth);
        let head_ms = head.map(|stamp| stamp.at.elapsed().as_secs_f64() * 1000.0);
        let window = json!({"queue":self.label,"window_ms":self.window.elapsed().as_secs_f64()*1000.0,"depth":depth,"highwater":self.highwater,"oldest_head_ms":head_ms,"head_class":head.map(|s| ["dtls","srtp","other"][s.class]),"dtls":self.samples[0].json(),"srtp":self.samples[1].json(),"other":self.samples[2].json(),"histogram_upper_ms":BOUNDS_MS});
        if self.window.elapsed() >= Duration::from_secs(5) {
            eprintln!("USB queue /5s: {window}");
            self.last = window.clone();
            self.window = Instant::now();
            self.samples = Default::default();
            self.highwater = depth;
        }
        json!({"current":window,"previous":self.last})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_mux_prefix_and_unknown_packets_are_never_claimed_as_dtls() {
        assert_eq!(packet_class(&[22], 4), 2);
        assert_eq!(packet_class(&[0, 1, 0, 2, 22], 4), 2);
        assert_eq!(packet_class(&[0x9c, 0x41, 0x9c, 0x42, 22], 4), 0);
        assert_eq!(packet_class(&[0x9c, 0x41, 0x9c, 0x42, 128], 4), 1);
        assert_eq!(packet_class(&[], 0), 2);
        assert_eq!(packet_class(&[255], 0), 2);
    }
    #[test]
    fn queue_head_age_and_highwater_survive_window_reset_without_cross_clock_math() {
        let mut trace = Trace::testing("test");
        trace.depth(8);
        let head = Stamp {
            at: Instant::now() - Duration::from_millis(75),
            class: 0,
        };
        trace.removed(Some(head));
        trace.window = Instant::now() - Duration::from_secs(6);
        let v = trace.snapshot(2, Some(head));
        assert_eq!(v["current"]["highwater"], 8);
        assert!(v["current"]["oldest_head_ms"].as_f64().unwrap() >= 75.0);
        assert_eq!(v["current"]["dtls"]["p95_bucket_upper_ms"], 100.0);
        let next = trace.snapshot(0, None);
        assert_eq!(next["current"]["highwater"], 2);
        assert_eq!(next["current"]["dtls"]["count"], 0);
        assert_eq!(next["previous"], v["current"]);
    }
    #[test]
    fn classes_and_histogram_are_bounded_and_preserve_tail_age() {
        assert_eq!(classify(Some(22)), 0);
        assert_eq!(classify(Some(128)), 1);
        assert_eq!(classify(Some(0)), 2);
        let mut samples = Samples::default();
        for ms in [1, 2, 5, 10, 3000] {
            samples.record(Duration::from_millis(ms));
        }
        let v = samples.json();
        assert_eq!(v["count"], 5);
        assert_eq!(v["max_ms"], 3000.0);
        assert_eq!(v["p95_bucket_upper_ms"], 3000.0);
        assert_eq!(samples.bins.iter().sum::<u64>(), 5);
    }
}
