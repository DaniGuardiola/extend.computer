//! Retry transport failures only. Consent, identity, protocol and local helper
//! errors terminate; successful user stop is never interpreted as reconnect.
use crate::error::is_connection_failure as retryable;
use anyhow::Result;
use std::time::Duration;
pub fn run(enabled: bool, action: impl FnMut() -> Result<()>) -> Result<()> {
    run_with_wait(enabled, action, std::thread::sleep)
}
fn run_with_wait(
    enabled: bool,
    mut action: impl FnMut() -> Result<()>,
    mut wait: impl FnMut(Duration),
) -> Result<()> {
    let mut failures = 0u32;
    loop {
        match action() {
            Ok(()) => return Ok(()),
            Err(error) if enabled && retryable(&error) => {
                failures = failures.saturating_add(1);
                let seconds = 1u64 << failures.min(3).saturating_sub(1);
                eprintln!("Connection lost/unavailable; local control restored. Retrying in {seconds}s. Ctrl-C stops.");
                wait(Duration::from_secs(seconds));
            }
            Err(error) => return Err(error),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_transport_failures_retry_and_user_stop_ends_loop() {
        let mut calls = 0;
        let mut waits = Vec::new();
        run_with_wait(
            true,
            || {
                calls += 1;
                if calls < 4 {
                    Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "test").into())
                } else {
                    Ok(())
                }
            },
            |d| waits.push(d.as_secs()),
        )
        .unwrap();
        assert_eq!(calls, 4);
        assert_eq!(waits, vec![1, 2, 4]);
    }
    #[test]
    fn denial_identity_change_and_protocol_errors_are_terminal() {
        for message in [
            "control denied",
            "peer identity changed",
            "invalid input",
            "capture stopped",
        ] {
            let mut calls = 0;
            assert!(run_with_wait(
                true,
                || {
                    calls += 1;
                    anyhow::bail!(message.to_string())
                },
                |_| panic!("must not retry")
            )
            .is_err());
            assert_eq!(calls, 1);
        }
    }
    #[test]
    fn classified_helper_failure_does_not_retry_its_underlying_io() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "helper pipe",
        ))
        .context(crate::error::EngineError::HelperUnavailable);
        assert!(!retryable(&error));
    }
    #[test]
    fn opt_out_does_not_retry_transport() {
        assert!(run_with_wait(
            false,
            || Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "test").into()),
            |_| panic!("must not retry")
        )
        .is_err());
    }
}
