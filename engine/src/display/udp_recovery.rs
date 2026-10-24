//! A short, monotonic grace period for an established, authenticated physical
//! USB route. Eligibility belongs to the desktop's IOKit NCM classification;
//! this state never enables itself from an address or a network error.
use std::{
    io,
    time::{Duration, Instant},
};

pub(super) const GRACE: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_millis(50);
const CONTROL_FRESHNESS: Duration = Duration::from_secs(3);

// Darwin errno values. This policy is enabled only for macOS UDP sockets.
const NET_UNREACHABLE: i32 = 51;
const HOST_UNREACHABLE: i32 = 65;

pub(super) fn route_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(NET_UNREACHABLE | HOST_UNREACHABLE)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    ControlUnavailable,
    Expired,
    EpochExhausted,
}

impl Failure {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::ControlUnavailable => "authenticated control heartbeat unavailable or stale",
            Self::Expired => "fixed USB route recovery grace expired",
            Self::EpochExhausted => "USB route recovery epoch exhausted",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Outage {
    since: Instant,
    retry_at: Instant,
    errno: i32,
}

#[derive(Default, Debug)]
pub(super) struct Recovery {
    control: Option<Instant>,
    outage: Option<Outage>,
    epoch: u64,
    ended: Option<Instant>,
}

impl Recovery {
    pub(super) fn update_control(&mut self, authenticated: Option<Instant>) {
        self.control = authenticated;
    }

    pub(super) fn active(&self) -> bool {
        self.outage.is_some()
    }

    pub(super) fn checkpoint(&self) -> (u64, Option<Instant>) {
        (self.epoch, self.ended)
    }

    fn control_fresh(&self, now: Instant) -> bool {
        self.control
            .and_then(|at| now.checked_duration_since(at))
            .is_some_and(|age| age < CONTROL_FRESHNESS)
    }

    pub(super) fn validate(&self, now: Instant) -> Result<(), Failure> {
        let Some(outage) = self.outage else {
            return Ok(());
        };
        if !self.control_fresh(now) {
            return Err(Failure::ControlUnavailable);
        }
        if now.duration_since(outage.since) >= GRACE {
            return Err(Failure::Expired);
        }
        Ok(())
    }

    pub(super) fn ready(&self, now: Instant) -> Result<bool, Failure> {
        self.validate(now)?;
        Ok(self.outage.is_none_or(|outage| now >= outage.retry_at))
    }

    /// Every attempted retry advances its schedule, including WouldBlock. The
    /// caller's usual send behavior is unchanged outside a route outage.
    pub(super) fn attempted(&mut self, now: Instant) {
        if let Some(outage) = &mut self.outage {
            outage.retry_at = now + RETRY;
        }
    }

    /// Return true only for the first failure of an outage. Heartbeats and
    /// further errors can move the next retry, never the original deadline.
    pub(super) fn failed(&mut self, error: &io::Error, now: Instant) -> Result<bool, Failure> {
        debug_assert!(route_error(error));
        self.validate(now)?;
        if !self.control_fresh(now) {
            return Err(Failure::ControlUnavailable);
        }
        let first = self.outage.is_none();
        if first {
            self.epoch = self.epoch.checked_add(1).ok_or(Failure::EpochExhausted)?;
            self.ended = None;
        }
        let outage = self.outage.get_or_insert(Outage {
            since: now,
            retry_at: now,
            errno: error.raw_os_error().unwrap(),
        });
        outage.retry_at = now + RETRY;
        Ok(first)
    }

    /// A successful send ends one outage; additional successful packets cannot
    /// request another recovery keyframe.
    pub(super) fn succeeded(&mut self, now: Instant) -> bool {
        if self.outage.take().is_some() {
            self.ended = Some(now);
            true
        } else {
            false
        }
    }

    pub(super) fn error(&self) -> io::Error {
        let errno = self.outage.map_or(HOST_UNREACHABLE, |outage| outage.errno);
        io::Error::from_raw_os_error(errno)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unreachable() -> io::Error {
        io::Error::from_raw_os_error(HOST_UNREACHABLE)
    }

    #[test]
    fn errors_and_fresh_heartbeats_never_extend_the_fixed_grace() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.update_control(Some(now));
        assert!(recovery.failed(&unreachable(), now).unwrap());
        assert!(!recovery
            .ready(now + RETRY - Duration::from_nanos(1))
            .unwrap());
        assert!(recovery.ready(now + RETRY).unwrap());
        for millis in [50, 500, 1_000, 1_950] {
            let at = now + Duration::from_millis(millis);
            recovery.update_control(Some(at));
            assert!(!recovery
                .failed(&io::Error::from_raw_os_error(NET_UNREACHABLE), at)
                .unwrap());
        }
        assert_eq!(recovery.ready(now + GRACE), Err(Failure::Expired));
        assert_eq!(
            recovery.failed(&unreachable(), now + GRACE),
            Err(Failure::Expired)
        );
    }

    #[test]
    fn one_success_clears_the_outage_and_requests_exactly_one_keyframe() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.update_control(Some(now));
        recovery.failed(&unreachable(), now).unwrap();
        assert!(recovery.active());
        assert!(recovery.succeeded(now + RETRY));
        assert!(!recovery.active());
        assert!(!recovery.succeeded(now + RETRY + RETRY));
        assert!(recovery.ready(now + GRACE).unwrap());
    }

    #[test]
    fn missing_stale_and_future_control_timestamps_cannot_enable_recovery() {
        let now = Instant::now();
        for control in [None, Some(now - CONTROL_FRESHNESS), Some(now + RETRY)] {
            let mut recovery = Recovery::default();
            recovery.update_control(control);
            assert_eq!(
                recovery.failed(&unreachable(), now),
                Err(Failure::ControlUnavailable)
            );
            assert!(!recovery.active());
        }
    }

    #[test]
    fn control_death_and_staleness_end_an_outage_before_another_send() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.update_control(Some(now - CONTROL_FRESHNESS + RETRY));
        recovery.failed(&unreachable(), now).unwrap();
        assert_eq!(
            recovery.ready(now + RETRY),
            Err(Failure::ControlUnavailable)
        );
        recovery.update_control(Some(now));
        assert!(recovery.ready(now + RETRY).unwrap());
        recovery.update_control(None);
        assert_eq!(
            recovery.ready(now + RETRY),
            Err(Failure::ControlUnavailable)
        );
    }

    #[test]
    fn only_the_two_darwin_route_errors_are_eligible() {
        assert!(route_error(&unreachable()));
        assert!(route_error(&io::Error::from_raw_os_error(NET_UNREACHABLE)));
        for errno in [9, 13, 22, 35, 49, 55, 60] {
            assert!(!route_error(&io::Error::from_raw_os_error(errno)));
        }
        assert!(!route_error(&io::Error::new(
            io::ErrorKind::WouldBlock,
            "queue full"
        )));
    }

    #[test]
    fn a_retry_that_would_block_still_waits_fifty_milliseconds() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.update_control(Some(now));
        recovery.failed(&unreachable(), now).unwrap();
        let retry = now + RETRY;
        assert!(recovery.ready(retry).unwrap());
        recovery.attempted(retry);
        assert!(!recovery.ready(retry + Duration::from_millis(49)).unwrap());
        assert!(recovery.ready(retry + RETRY).unwrap());
    }

    #[test]
    fn terminal_failure_preserves_the_original_route_errno() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.update_control(Some(now));
        recovery
            .failed(&io::Error::from_raw_os_error(NET_UNREACHABLE), now)
            .unwrap();
        assert_eq!(recovery.error().raw_os_error(), Some(NET_UNREACHABLE));
    }

    #[test]
    fn checkpoint_retains_short_outages_and_only_new_failures_advance_epoch() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        assert_eq!(recovery.checkpoint(), (0, None));
        recovery.update_control(Some(now));
        recovery.failed(&unreachable(), now).unwrap();
        assert_eq!(recovery.checkpoint(), (1, None));
        recovery.failed(&unreachable(), now + RETRY).unwrap();
        assert_eq!(recovery.checkpoint(), (1, None));
        let ended = now + RETRY + RETRY;
        assert!(recovery.succeeded(ended));
        assert!(!recovery.active());
        assert_eq!(recovery.checkpoint(), (1, Some(ended)));
        assert!(!recovery.succeeded(ended + RETRY));
        assert_eq!(recovery.checkpoint(), (1, Some(ended)));
        recovery.failed(&unreachable(), ended + RETRY).unwrap();
        assert_eq!(recovery.checkpoint(), (2, None));
    }
}
