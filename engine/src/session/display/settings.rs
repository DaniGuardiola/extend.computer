//! A session-local, single-request receiver settings mailbox.
use super::Config;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub auto_size: bool,
    pub fps: u32,
    pub bitrate: u32,
}
impl Settings {
    pub fn apply(&self, current: &Config) -> Result<Config> {
        let mut next = current.clone();
        next.width = self.width;
        next.height = self.height;
        next.auto_size = self.auto_size;
        next.fps = self.fps;
        next.bitrate = self.bitrate;
        next.validate()?;
        Ok(next)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub expected_revision: u64,
    pub settings: Settings,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    Rejected,
    Stale,
    Failed,
    Closed,
}
#[derive(Debug)]
pub struct Rejected;
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Display settings rejected")
    }
}
impl std::error::Error for Rejected {}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub id: u64,
    pub revision: u64,
    pub outcome: Outcome,
}
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub supported: bool,
    pub closed: bool,
    pub revision: u64,
    pub config: Config,
    pub pending: Option<u64>,
    pub result: Option<Completion>,
}
struct State {
    view: Snapshot,
    next_id: u64,
    queued: Option<Request>,
    expected_revision: u64,
}
pub struct ReceiverSettings {
    state: Mutex<State>,
}
impl ReceiverSettings {
    pub fn new(supported: bool, config: Config) -> Self {
        Self {
            state: Mutex::new(State {
                view: Snapshot {
                    supported,
                    closed: false,
                    revision: 0,
                    config,
                    pending: None,
                    result: None,
                },
                next_id: 0,
                queued: None,
                expected_revision: 0,
            }),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().view.clone()
    }
    pub fn request(&self, expected_revision: u64, settings: Settings) -> Result<u64> {
        let mut state = self.state.lock().unwrap();
        ensure!(state.view.supported, "Receiver settings are unsupported");
        ensure!(!state.view.closed, "Display ended");
        ensure!(state.view.pending.is_none(), "Display settings are pending");
        ensure!(
            expected_revision == state.view.revision,
            "Display settings changed"
        );
        settings.apply(&state.view.config)?;
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Display settings request limit"))?;
        let id = state.next_id;
        state.view.pending = Some(id);
        state.view.result = None;
        state.expected_revision = expected_revision;
        state.queued = Some(Request {
            id,
            expected_revision,
            settings,
        });
        Ok(id)
    }
    pub(super) fn take(&self) -> Option<Request> {
        self.state.lock().unwrap().queued.take()
    }
    pub(super) fn configured(&self, revision: u64, config: &Config) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        ensure!(
            !state.view.closed && revision == state.view.revision.checked_add(1).unwrap_or(0),
            "Stale display configuration"
        );
        state.view.revision = revision;
        state.view.config = config.clone();
        Ok(())
    }
    pub(super) fn complete(&self, result: Completion) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        ensure!(
            !state.view.closed && state.view.pending == Some(result.id),
            "Stale display settings result"
        );
        ensure!(
            result.revision == state.view.revision,
            "Invalid display settings revision"
        );
        ensure!(
            result.outcome != Outcome::Applied || result.revision > state.expected_revision,
            "Source did not apply display settings"
        );
        state.view.pending = None;
        state.queued = None;
        state.view.result = Some(result);
        Ok(())
    }
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.view.closed = true;
        state.queued = None;
        if let Some(id) = state.view.pending.take() {
            state.view.result = Some(Completion {
                id,
                revision: state.view.revision,
                outcome: Outcome::Closed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            mode: super::super::Mode::Extend,
            width: 1280,
            height: 800,
            fps: 60,
            bitrate: 20_000_000,
            retina: false,
            auto_size: false,
            codec: super::super::VideoCodec::Hevc,
            separate_cursor: true,
        }
    }
    fn settings() -> Settings {
        Settings {
            width: 960,
            height: 600,
            auto_size: false,
            fps: 30,
            bitrate: 8_000_000,
        }
    }
    #[test]
    fn pending_stale_unsupported_and_closed_requests_fail_without_mutation() {
        let unsupported = ReceiverSettings::new(false, config());
        assert!(unsupported.request(0, settings()).is_err());
        let mailbox = ReceiverSettings::new(true, config());
        assert!(mailbox.request(1, settings()).is_err());
        let id = mailbox.request(0, settings()).unwrap();
        assert!(mailbox.request(0, settings()).is_err());
        assert_eq!(mailbox.snapshot().config, config());
        assert_eq!(mailbox.take().unwrap().id, id);
        assert!(mailbox.take().is_none());
        mailbox.close();
        assert_eq!(mailbox.snapshot().result.unwrap().outcome, Outcome::Closed);
        assert!(mailbox.request(0, settings()).is_err());
    }
    #[test]
    fn decoder_prepared_is_not_source_applied_and_wrong_results_are_rejected() {
        let mailbox = ReceiverSettings::new(true, config());
        let id = mailbox.request(0, settings()).unwrap();
        assert!(mailbox
            .complete(Completion {
                id,
                revision: 0,
                outcome: Outcome::Applied
            })
            .is_err());
        let next = settings().apply(&config()).unwrap();
        assert_eq!(next.mode, config().mode);
        assert_eq!(next.codec, config().codec);
        assert_eq!(next.retina, config().retina);
        assert_eq!(next.separate_cursor, config().separate_cursor);
        mailbox.configured(1, &next).unwrap();
        assert_eq!(mailbox.snapshot().pending, Some(id));
        assert!(mailbox.snapshot().result.is_none());
        assert!(mailbox
            .complete(Completion {
                id: id + 1,
                revision: 1,
                outcome: Outcome::Applied
            })
            .is_err());
        mailbox
            .complete(Completion {
                id,
                revision: 1,
                outcome: Outcome::Applied,
            })
            .unwrap();
        assert_eq!(mailbox.snapshot().pending, None);
        assert!(mailbox.request(0, settings()).is_err());
    }
}
