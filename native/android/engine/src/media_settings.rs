//! Receiver UI completion is later than signaling's decoder-prepared ACK.
use anyhow::{ensure, Result};
use extend_computer_agent::session::display::{
    settings::{Outcome, Snapshot},
    Config,
};
use serde::Serialize;
use std::time::{Duration, Instant};

const APPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Serialize)]
pub(super) struct Applied {
    pub revision: u64,
    pub config: Config,
}
#[derive(Clone, Serialize)]
pub(super) struct Completion {
    pub id: u64,
    pub revision: u64,
    pub outcome: &'static str,
    pub error: Option<&'static str>,
}
struct Pending {
    id: u64,
    started: Instant,
    // Count only validated decoder callbacks received AFTER source confirmation.
    // This is not a cross-device timestamp or a media provenance marker.
    source: Option<(u64, u64)>,
}
#[derive(Default)]
pub(super) struct ApplyGate {
    applied: Option<Applied>,
    pending: Option<Pending>,
    result: Option<Completion>,
    closed: bool,
    timed_out: bool,
    failed: bool,
}
impl ApplyGate {
    pub fn can_request(&self) -> Result<()> {
        ensure!(!self.closed, "Display ended");
        ensure!(
            !self.timed_out && !self.failed,
            "Reconnect to change display settings"
        );
        ensure!(self.pending.is_none(), "Display settings are pending");
        Ok(())
    }
    pub fn started(&mut self, id: u64, now: Instant) {
        self.pending = Some(Pending {
            id,
            started: now,
            source: None,
        });
        self.result = None;
    }
    fn finish(&mut self, revision: u64, outcome: &'static str, error: Option<&'static str>) {
        if let Some(pending) = self.pending.take() {
            self.result = Some(Completion {
                id: pending.id,
                revision,
                outcome,
                error,
            });
        }
    }
    pub fn close(&mut self, revision: u64) {
        self.closed = true;
        self.finish(
            revision,
            "closed",
            Some("Display ended before settings were applied."),
        );
    }
    pub fn observe(
        &mut self,
        signal: &Snapshot,
        revision: u64,
        config: &Config,
        rendered: u64,
        streaming: bool,
        now: Instant,
    ) {
        if signal.closed {
            self.close(revision);
        }
        if self.closed || self.timed_out || self.failed {
            return;
        }
        if let Some(pending) = self.pending.as_ref() {
            if now.saturating_duration_since(pending.started) >= APPLY_TIMEOUT {
                self.timed_out = true;
                self.finish(
                    revision,
                    "timed_out",
                    Some("Settings timed out. Reconnect to change settings."),
                );
                return;
            }
            if let Some(result) = signal
                .result
                .as_ref()
                .filter(|result| result.id == pending.id)
            {
                match result.outcome {
                    Outcome::Applied => {
                        if result.revision == revision
                            && signal.revision == revision
                            && signal.config == *config
                        {
                            let pending = self.pending.as_mut().unwrap();
                            let (_, baseline) = *pending.source.get_or_insert((revision, rendered));
                            if pending
                                .source
                                .is_some_and(|(source_revision, _)| source_revision == revision)
                                && streaming
                                && rendered > baseline
                            {
                                self.applied = Some(Applied {
                                    revision,
                                    config: config.clone(),
                                });
                                self.finish(revision, "applied", None);
                            }
                        }
                    }
                    Outcome::Rejected => self.finish(
                        revision,
                        "rejected",
                        Some("The other device couldn't apply these settings."),
                    ),
                    Outcome::Stale => self.finish(
                        revision,
                        "stale",
                        Some(
                            "Display settings changed. Review the current settings and try again.",
                        ),
                    ),
                    Outcome::Failed => {
                        self.failed = true;
                        self.finish(revision, "failed",
                            Some("The display couldn't apply these settings. Reconnect to try again."));
                    }
                    Outcome::Closed => self.close(revision),
                }
            }
            return;
        }
        if streaming
            && rendered > 0
            && (!signal.supported || (signal.revision == revision && signal.config == *config))
            && self
                .applied
                .as_ref()
                .is_none_or(|applied| applied.revision != revision)
        {
            self.applied = Some(Applied {
                revision,
                config: config.clone(),
            });
        }
    }
    pub fn snapshot(
        &self,
        signal: &Snapshot,
        id: &str,
        revision: u64,
        config: &Config,
        streaming: bool,
    ) -> serde_json::Value {
        serde_json::json!({"id":id,"supported":signal.supported,"closed":self.closed || signal.closed,
            "available":signal.supported && !self.closed && !signal.closed && !self.timed_out && !self.failed && streaming,
            "revision":revision,"config":config,"applied":self.applied,
            "pending":self.pending.as_ref().map(|pending|pending.id),"result":self.result})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::session::display::{
        settings::Completion as SourceCompletion, Mode, VideoCodec,
    };
    fn config() -> Config {
        Config {
            mode: Mode::Extend,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 20_000_000,
            retina: false,
            auto_size: false,
            codec: VideoCodec::Hevc,
            separate_cursor: true,
        }
    }
    fn signal() -> Snapshot {
        Snapshot {
            supported: true,
            closed: false,
            revision: 0,
            config: config(),
            pending: None,
            result: None,
        }
    }
    #[test]
    fn source_ack_requires_a_later_validated_frame_and_blocks_a_second_request() {
        let now = Instant::now();
        let mut gate = ApplyGate::default();
        let mut signal = signal();
        gate.observe(&signal, 0, &signal.config, 1, true, now);
        gate.started(3, now);
        signal.revision = 1;
        signal.config.fps = 30;
        signal.pending = Some(3);
        gate.observe(&signal, 1, &signal.config, 9, true, now);
        assert!(gate.can_request().is_err());
        assert_eq!(gate.applied.as_ref().unwrap().revision, 0);
        signal.pending = None;
        signal.result = Some(SourceCompletion {
            id: 3,
            revision: 1,
            outcome: Outcome::Applied,
        });
        gate.observe(&signal, 1, &signal.config, 9, true, now);
        assert!(gate.pending.is_some()); // Existing render callbacks cannot certify the source barrier.
        gate.observe(&signal, 0, &config(), 100, true, now);
        assert!(gate.pending.is_some());
        gate.observe(&signal, 1, &signal.config, 10, false, now);
        assert!(gate.pending.is_some());
        gate.observe(&signal, 1, &signal.config, 10, true, now);
        assert!(gate.can_request().is_ok());
        assert_eq!(gate.result.as_ref().unwrap().outcome, "applied");
        assert_eq!(gate.applied.as_ref().unwrap().config.fps, 30);
    }
    #[test]
    fn rejection_and_close_keep_previous_applied_settings() {
        let now = Instant::now();
        let mut gate = ApplyGate::default();
        let mut signal = signal();
        gate.observe(&signal, 0, &signal.config, 1, true, now);
        gate.started(1, now);
        signal.result = Some(SourceCompletion {
            id: 99,
            revision: 0,
            outcome: Outcome::Rejected,
        });
        gate.observe(&signal, 0, &signal.config, 2, true, now);
        assert!(gate.pending.is_some());
        signal.result.as_mut().unwrap().id = 1;
        gate.observe(&signal, 0, &signal.config, 3, true, now);
        assert_eq!(gate.result.as_ref().unwrap().outcome, "rejected");
        assert_eq!(gate.applied.as_ref().unwrap().revision, 0);
        gate.started(2, now);
        gate.close(0);
        assert!(gate.can_request().is_err());
        assert_eq!(gate.result.as_ref().unwrap().outcome, "closed");
    }
    #[test]
    fn timeout_is_bounded_and_late_success_cannot_resurrect_the_request() {
        let now = Instant::now();
        let mut gate = ApplyGate::default();
        let mut signal = signal();
        gate.started(1, now);
        gate.observe(&signal, 0, &signal.config, 1, true, now + APPLY_TIMEOUT);
        assert!(gate.pending.is_none());
        assert!(gate.can_request().is_err());
        assert_eq!(gate.result.as_ref().unwrap().outcome, "timed_out");
        signal.revision = 1;
        signal.result = Some(SourceCompletion {
            id: 1,
            revision: 1,
            outcome: Outcome::Applied,
        });
        gate.observe(
            &signal,
            1,
            &signal.config,
            100,
            true,
            now + APPLY_TIMEOUT + Duration::from_secs(1),
        );
        assert_eq!(gate.result.as_ref().unwrap().outcome, "timed_out");
        assert!(gate.applied.is_none());
    }
    #[test]
    fn failed_source_barrier_cannot_publish_early_receiver_output_as_applied() {
        let now = Instant::now();
        let mut gate = ApplyGate::default();
        let mut signal = signal();
        gate.observe(&signal, 0, &signal.config, 1, true, now);
        gate.started(1, now);
        signal.revision = 1;
        signal.config.fps = 30;
        signal.result = Some(SourceCompletion {
            id: 1,
            revision: 1,
            outcome: Outcome::Failed,
        });
        gate.observe(&signal, 1, &signal.config, 8, true, now);
        gate.observe(&signal, 1, &signal.config, 9, true, now);
        assert_eq!(gate.applied.as_ref().unwrap().revision, 0);
        assert_eq!(gate.result.as_ref().unwrap().outcome, "failed");
        assert!(gate.can_request().is_err());
    }
}
