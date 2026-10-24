//! Only operational broker health receives a short, bounded grace period.
//! Registration and Accessibility are checked separately on every refresh.
use std::time::{Duration, Instant};

const FRESH: Duration = Duration::from_secs(3);
const GRACE: Duration = Duration::from_secs(10);
#[derive(Default)]
pub(super) struct Readiness {
    checked: Option<Instant>,
    succeeded: Option<Instant>,
    failures: u8,
}
impl Readiness {
    pub const fn new() -> Self {
        Self {
            checked: None,
            succeeded: None,
            failures: 0,
        }
    }
    pub fn cached(&self, now: Instant) -> Option<bool> {
        self.checked
            .filter(|t| now.duration_since(*t) < FRESH)
            .map(|_| self.allowed(now))
    }
    fn allowed(&self, now: Instant) -> bool {
        self.failures < 2
            && self
                .succeeded
                .is_some_and(|t| now.duration_since(t) <= GRACE)
    }
    pub fn record(&mut self, now: Instant, success: bool) -> bool {
        self.checked = Some(now);
        if success {
            self.succeeded = Some(now);
            self.failures = 0;
        } else {
            self.failures = self.failures.saturating_add(1);
        }
        self.allowed(now)
    }
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }
}

/// Serializes operational probes, but authoritative denial never waits for one.
pub(super) struct Gate {
    cache: std::sync::Mutex<Readiness>,
    revoked: std::sync::atomic::AtomicBool,
}
impl Gate {
    pub const fn new() -> Self {
        Self {
            cache: std::sync::Mutex::new(Readiness::new()),
            revoked: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn check(
        &self,
        registration: i32,
        probe_allowed: bool,
        probe: impl FnOnce() -> bool,
    ) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        if registration != 1 {
            self.revoked.store(true, SeqCst);
            return false;
        }
        if !probe_allowed {
            return !self.revoked.load(SeqCst)
                && self
                    .cache
                    .try_lock()
                    .ok()
                    .and_then(|cache| cache.cached(Instant::now()))
                    .unwrap_or(false);
        }
        let mut cache = self.cache.lock().unwrap();
        if self.revoked.swap(false, SeqCst) {
            cache.invalidate();
        }
        if let Some(value) = cache.cached(Instant::now()) {
            return value;
        }
        let success = probe();
        if self.revoked.load(SeqCst) {
            cache.invalidate();
            return false;
        }
        cache.record(Instant::now(), success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_transient_recovers_but_two_failures_revoke() {
        let t = Instant::now();
        let mut r = Readiness::new();
        assert!(!r.record(t, false)); // Never grant without actual success.
        assert!(r.record(t, true));
        assert_eq!(r.cached(t + Duration::from_secs(2)), Some(true));
        assert_eq!(r.cached(t + Duration::from_secs(3)), None);
        assert!(r.record(t + Duration::from_secs(4), false));
        assert!(r.record(t + Duration::from_secs(7), true));
        assert!(r.record(t + Duration::from_secs(8), false));
        assert!(!r.record(t + Duration::from_secs(9), false));
    }
    #[test]
    fn grace_expires_even_inside_cached_failure_and_registration_resets_it() {
        let t = Instant::now();
        let mut r = Readiness::new();
        r.record(t, true);
        assert!(r.record(t + Duration::from_secs(9), false));
        assert_eq!(r.cached(t + Duration::from_secs(11)), Some(false));
        r.invalidate();
        assert_eq!(r.cached(t + Duration::from_secs(11)), None);
        assert!(!r.record(t + Duration::from_secs(11), false));
    }
    #[test]
    fn overlapping_callers_share_one_probe() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc,
        };
        let gate = Arc::new(Gate::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let g = gate.clone();
        let c = calls.clone();
        let first = std::thread::spawn(move || {
            g.check(1, true, || {
                c.fetch_add(1, Ordering::SeqCst);
                started.send(()).unwrap();
                wait.recv().unwrap();
                true
            })
        });
        ready.recv().unwrap();
        let g = gate.clone();
        let c = calls.clone();
        let second = std::thread::spawn(move || {
            g.check(1, true, || {
                c.fetch_add(1, Ordering::SeqCst);
                true
            })
        });
        release.send(()).unwrap();
        assert!(first.join().unwrap());
        assert!(second.join().unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn registration_denial_bypasses_running_probe_and_discards_its_success() {
        use std::sync::{mpsc, Arc};
        let gate = Arc::new(Gate::new());
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let g = gate.clone();
        let running = std::thread::spawn(move || {
            g.check(1, true, || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                true
            })
        });
        ready.recv().unwrap();
        let (done, result) = mpsc::channel();
        let g = gate.clone();
        let denied = std::thread::spawn(move || {
            done.send(g.check(2, true, || panic!("No probe for denial")))
                .unwrap()
        });
        let immediate = result.recv_timeout(Duration::from_secs(1));
        release.send(()).unwrap();
        assert_eq!(immediate.unwrap(), false);
        denied.join().unwrap();
        assert!(!running.join().unwrap());
        assert!(!gate.check(1, true, || false)); // Old success never becomes grace.
    }
}
