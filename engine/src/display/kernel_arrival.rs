//! Optional Mach-clock UDP arrival times for RTC; no wall clock conversion.
use super::{metrics, socket_timing};
use std::{
    io,
    net::UdpSocket,
    time::{Duration, Instant},
};

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}
extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
}

#[derive(Clone, Copy)]
struct Clock {
    numer: u32,
    denom: u32,
}

impl Clock {
    fn new() -> io::Result<Self> {
        let mut info = Timebase { numer: 0, denom: 0 };
        // The initialized output buffer remains valid for the synchronous call.
        let status = unsafe { mach_timebase_info(&mut info) };
        if status != 0 || info.numer == 0 || info.denom == 0 {
            return Err(io::Error::other("Mach clock timebase unavailable"));
        }
        Ok(Self {
            numer: info.numer,
            denom: info.denom,
        })
    }

    fn convert(&self, arrival_ticks: u64, now_ticks: u64, now: Instant) -> Option<Instant> {
        if arrival_ticks == 0 || self.numer == 0 || self.denom == 0 {
            return None;
        }
        // Subtract raw ticks before applying the rational timebase. u128 keeps
        // multiplication exact even for large uptimes/timebase numerators.
        let elapsed_ticks = now_ticks.checked_sub(arrival_ticks)?;
        let nanos = u64::try_from(
            u128::from(elapsed_ticks) * u128::from(self.numer) / u128::from(self.denom),
        )
        .ok()?;
        now.checked_sub(Duration::from_nanos(nanos))
    }

    fn arrival(&self, arrival_ticks: Option<u64>) -> Option<Instant> {
        let arrival_ticks = arrival_ticks?;
        // Map a fresh local Mach/Instant pair, rather than assuming Instant's
        // implementation/epoch or relating it to an adjustable wall clock.
        // The midpoint bounds the clock-sampling error to this tiny bracket.
        let before = unsafe { mach_absolute_time() };
        let now = Instant::now();
        let after = unsafe { mach_absolute_time() };
        let middle = before.checked_add(after.checked_sub(before)? / 2)?;
        self.convert(arrival_ticks, middle, now)
    }
}

pub(super) struct Receiver {
    clock: Option<Clock>,
    applied: u64,
    fallback: u64,
    last_report: Option<Instant>,
    reported: (u64, u64),
}

impl Receiver {
    pub fn new(socket: Option<&UdpSocket>, automatic: bool) -> Self {
        let clock = (|| {
            let socket = socket.ok_or_else(|| io::Error::other("Carrier is not a UDP socket"))?;
            let clock = Clock::new()?;
            socket_timing::enable_monotonic(socket)?;
            Ok::<_, io::Error>(clock)
        })();
        let clock = match clock {
            Ok(clock) => Some(clock),
            Err(error) => {
                eprintln!(
                    "Display kernel arrival unavailable; using userspace receive time: {error}"
                );
                None
            }
        };
        metrics::kernel_arrival_configuration(clock.is_some(), automatic);
        eprintln!(
            "Display receive clock: requested=kernel-monotonic actual={} fallback=userspace",
            if clock.is_some() {
                "kernel-monotonic"
            } else {
                "userspace"
            }
        );
        Self {
            clock,
            applied: 0,
            fallback: 0,
            last_report: None,
            reported: (0, 0),
        }
    }

    pub fn active(&self) -> bool {
        self.clock.is_some()
    }

    #[cfg(test)]
    pub fn counts(&self) -> (u64, u64) {
        (self.applied, self.fallback)
    }

    pub fn receive_time(&self, timestamps: socket_timing::Timestamps) -> (Instant, bool) {
        match self
            .clock
            .and_then(|clock| clock.arrival(timestamps.monotonic_ticks))
        {
            Some(arrival) => (arrival, true),
            None => (Instant::now(), false),
        }
    }

    // Count only accepted RTC packets, so the receipt proves times actually
    // supplied to RTC rather than unrelated packets observed on the socket.
    pub fn record(&mut self, applied: bool, now: Instant) {
        if applied {
            self.applied = self.applied.saturating_add(1);
        } else {
            self.fallback = self.fallback.saturating_add(1);
        }
        if self
            .last_report
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(1))
        {
            self.report();
            self.last_report = Some(now);
        }
    }

    fn report(&mut self) {
        metrics::kernel_arrival_counts(self.applied, self.fallback);
        eprintln!(
            "Display receive clock packets: kernel={} fallback={}",
            self.applied, self.fallback
        );
        self.reported = (self.applied, self.fallback);
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if self.reported != (self.applied, self.fallback) {
            self.report();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_ticks_use_exact_rational_timebase_without_u64_multiply_overflow() {
        let now = Instant::now();
        let clock = Clock {
            numer: 125,
            denom: 3,
        };
        let arrival = clock.convert(u64::MAX - 48, u64::MAX, now).unwrap();
        assert_eq!(now.duration_since(arrival), Duration::from_nanos(2_000));
        assert_eq!(
            clock.convert(100, 101, now).unwrap(),
            now - Duration::from_nanos(41)
        );
    }

    #[test]
    fn invalid_zero_future_and_unrepresentable_timestamps_fall_back() {
        let now = Instant::now();
        let clock = Clock {
            numer: 125,
            denom: 3,
        };
        assert!(clock.convert(0, 100, now).is_none());
        assert!(clock.convert(101, 100, now).is_none());
        assert!(Clock { numer: 1, denom: 0 }.convert(1, 2, now).is_none());
        assert!(Clock { numer: 0, denom: 1 }.convert(1, 2, now).is_none());
        assert!(Clock {
            numer: u32::MAX,
            denom: 1
        }
        .convert(1, u64::MAX, now)
        .is_none());
        let receiver = Receiver {
            clock: Some(clock),
            applied: 0,
            fallback: 0,
            last_report: None,
            reported: (0, 0),
        };
        for ticks in [None, Some(0), Some(u64::MAX)] {
            let before = Instant::now();
            let (arrival, applied) = receiver.receive_time(socket_timing::Timestamps {
                wall_ns: Some(u64::MAX),
                monotonic_ticks: ticks,
            });
            assert!(!applied);
            assert!((before..=Instant::now()).contains(&arrival));
        }
    }

    #[test]
    fn wall_clock_jumps_do_not_change_monotonic_receive_time() {
        let now = Instant::now();
        let clock = Clock {
            numer: 125,
            denom: 3,
        };
        let times = [Some(0), Some(1_000_000), Some(u64::MAX), None].map(|wall_ns| {
            let stamps = socket_timing::Timestamps {
                wall_ns,
                monotonic_ticks: Some(24),
            };
            // Deliberately varied wall stamps never enter the conversion.
            clock
                .convert(stamps.monotonic_ticks.unwrap(), 48, now)
                .unwrap()
        });
        assert!(times
            .iter()
            .all(|time| *time == now - Duration::from_nanos(1_000)));
    }

    #[test]
    fn unsupported_mode_counts_userspace_fallback_without_claiming_kernel_packets() {
        let mut receiver = Receiver::new(None, false);
        assert!(!receiver.active());
        let (arrival, applied) = receiver.receive_time(socket_timing::Timestamps::default());
        assert!(!applied);
        receiver.record(applied, arrival);
        assert_eq!((receiver.applied, receiver.fallback), (0, 1));
    }
}
