//! Live connection quality from a session's recent measurements.
use serde::Serialize;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Measurements older than this no longer describe the connection.
const WINDOW: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Good,
    Fair,
    Poor,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Quality {
    pub level: Level,
    /// Typical (90th percentile) round trip over the last few seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    /// Media packets lost, as a percentage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_percent: Option<f32>,
    /// How long the connection has been below good, for suggestions.
    pub degraded_seconds: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Sample {
    /// An acknowledged round trip.
    RoundTrip(Duration),
    /// Time between the peer's keepalives, which arrive at least every 500 ms.
    Gap(Duration),
    /// Fraction of media packets lost (0 to 1).
    Loss(f32),
    /// The video stream had to restart from a keyframe after losing data.
    Recovery,
}

#[derive(Default)]
pub(super) struct Tracker {
    samples: VecDeque<(Instant, Sample)>,
    degraded_since: Option<Instant>,
}

impl Tracker {
    pub(super) fn record(&mut self, sample: Sample) {
        self.record_at(Instant::now(), sample);
    }

    fn record_at(&mut self, now: Instant, sample: Sample) {
        self.samples.push_back((now, sample));
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > WINDOW)
        {
            self.samples.pop_front();
        }
        // Bound memory if a fast input stream records many round trips.
        while self.samples.len() > 512 {
            self.samples.pop_front();
        }
        let degraded = self.measure(now).is_some_and(|q| q.level != Level::Good);
        match (degraded, self.degraded_since) {
            (true, None) => self.degraded_since = Some(now),
            (false, Some(_)) => self.degraded_since = None,
            _ => {}
        }
    }

    /// Measured up to the latest sample: a session that stops sending (an
    /// idle mouse) keeps the quality it last had instead of going blank, and
    /// its degradation is only timed while measurements keep arriving.
    pub(super) fn view(&self) -> Option<Quality> {
        let latest = self.samples.back()?.0;
        let mut quality = self.measure(latest)?;
        quality.degraded_seconds = self
            .degraded_since
            .filter(|_| quality.level != Level::Good)
            .map_or(0, |since| latest.duration_since(since).as_secs() as u32);
        Some(quality)
    }

    fn measure(&self, now: Instant) -> Option<Quality> {
        let recent = || {
            self.samples
                .iter()
                .filter(move |(at, _)| now.duration_since(*at) <= WINDOW)
                .map(|(_, sample)| *sample)
        };
        let mut trips: Vec<_> = recent()
            .filter_map(|s| match s {
                Sample::RoundTrip(d) => Some(d),
                _ => None,
            })
            .collect();
        trips.sort();
        let latency =
            (!trips.is_empty()).then(|| trips[(trips.len() * 9 / 10).min(trips.len() - 1)]);
        let gap = recent()
            .filter_map(|s| match s {
                Sample::Gap(d) => Some(d),
                _ => None,
            })
            .max();
        let loss = recent()
            .filter_map(|s| match s {
                Sample::Loss(l) => Some(l),
                _ => None,
            })
            .last();
        let recoveries = recent().filter(|s| matches!(s, Sample::Recovery)).count();
        let mut levels = Vec::new();
        if let Some(latency) = latency {
            levels.push(grade(latency.as_millis(), 40, 100));
        }
        if let Some(gap) = gap {
            levels.push(grade(gap.as_millis(), 1000, 2500));
        }
        if let Some(loss) = loss {
            levels.push(grade((loss * 1000.) as u128, 10, 50));
        }
        if recoveries > 0 || !levels.is_empty() {
            levels.push(grade(recoveries as u128, 1, 3));
        }
        Some(Quality {
            level: levels.into_iter().max()?,
            latency_ms: latency.map(|d| d.as_millis().min(u32::MAX as u128) as u32),
            loss_percent: loss.map(|l| (l * 1000.).round() / 10.),
            degraded_seconds: 0,
        })
    }
}

/// Good below `fair`, fair below `poor`, poor otherwise.
fn grade(value: u128, fair: u128, poor: u128) -> Level {
    if value < fair {
        Level::Good
    } else if value < poor {
        Level::Fair
    } else {
        Level::Poor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn no_measurements_means_no_quality_yet() {
        assert_eq!(Tracker::default().view(), None);
    }

    #[test]
    fn typical_latency_ignores_a_single_spike() {
        let mut tracker = Tracker::default();
        let start = Instant::now();
        for i in 0..19 {
            tracker.record_at(start + ms(i * 100), Sample::RoundTrip(ms(8)));
        }
        tracker.record_at(start + ms(1900), Sample::RoundTrip(ms(300)));
        let quality = tracker.view().unwrap();
        assert_eq!(quality.level, Level::Good);
        assert_eq!(quality.latency_ms, Some(8));
    }

    #[test]
    fn the_worst_signal_decides_and_degradation_is_timed() {
        let mut tracker = Tracker::default();
        let start = Instant::now();
        tracker.record_at(start, Sample::RoundTrip(ms(10)));
        tracker.record_at(start + ms(100), Sample::Loss(0.02));
        assert_eq!(tracker.view().unwrap().level, Level::Fair);
        tracker.record_at(start + ms(3100), Sample::Gap(ms(3000)));
        let quality = tracker.view().unwrap();
        assert_eq!(quality.level, Level::Poor);
        assert_eq!(quality.loss_percent, Some(2.0));
        assert_eq!(quality.degraded_seconds, 3);
    }

    #[test]
    fn an_idle_session_keeps_its_last_quality() {
        let mut tracker = Tracker::default();
        let start = Instant::now();
        tracker.record_at(start, Sample::RoundTrip(ms(9)));
        let quality = tracker.view().unwrap();
        assert_eq!(quality.level, Level::Good);
        assert_eq!(quality.latency_ms, Some(9));
    }
    #[test]
    fn repeated_recoveries_are_poor_and_old_samples_expire() {
        let mut tracker = Tracker::default();
        let start = Instant::now();
        for i in 0..3 {
            tracker.record_at(start + ms(i * 500), Sample::Recovery);
        }
        assert_eq!(tracker.view().unwrap().level, Level::Poor);
        tracker.record_at(start + ms(7000), Sample::RoundTrip(ms(5)));
        let quality = tracker.view().unwrap();
        assert_eq!(quality.level, Level::Good);
        assert_eq!(quality.degraded_seconds, 0);
    }
}
