//! Stable encrypted compatibility envelope, independent of application SemVer.
use crate::{error::EngineError, wire::Channel};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const MIN_VERSION: u16 = 1;
pub const MAX_VERSION: u16 = 1;
pub const INPUT: &str = "input-v1";
pub const CURSOR: &str = "cursor-v1";
pub const CONTROL: &str = "control-v1";

// Keep this envelope additive and bounded. Unknown fields are deliberately
// accepted here; ordinary control messages remain strict and versioned.
#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    min_protocol: u16,
    max_protocol: u16,
    app_version: String,
    capabilities: Vec<String>,
}
fn negotiate(local: &Hello, remote: &Hello) -> Result<(u16, Vec<String>)> {
    ensure!(
        remote.min_protocol > 0
            && remote.min_protocol <= remote.max_protocol
            && remote.app_version.len() <= 64
            && remote.capabilities.len() <= 32
            && remote.capabilities.iter().all(|s| s.len() <= 64),
        EngineError::ProtocolIncompatible
    );
    let version = local.max_protocol.min(remote.max_protocol);
    ensure!(
        version >= local.min_protocol.max(remote.min_protocol),
        EngineError::ProtocolIncompatible
    );
    let capabilities = local
        .capabilities
        .iter()
        .filter(|c| remote.capabilities.contains(c))
        .cloned()
        .collect();
    Ok((version, capabilities))
}
pub(crate) fn exchange(channel: &mut Channel) -> Result<()> {
    let local = Hello {
        min_protocol: MIN_VERSION,
        max_protocol: MAX_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: [INPUT, CURSOR, CONTROL]
            .into_iter()
            .map(String::from)
            .collect(),
    };
    channel.send(&local)?;
    let remote: Hello = channel.receive()?;
    let (version, capabilities) = negotiate(&local, &remote)?;
    channel.set_protocol(version, capabilities);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hello(min: u16, max: u16, app: &str, capabilities: &[&str]) -> Hello {
        Hello {
            min_protocol: min,
            max_protocol: max,
            app_version: app.into(),
            capabilities: capabilities.iter().map(|c| (*c).into()).collect(),
        }
    }
    #[test]
    fn app_versions_do_not_need_to_match() {
        let a = hello(1, 3, "2.0.0", &[INPUT, CONTROL]);
        let b = hello(1, 2, "0.9.0", &[INPUT]);
        assert_eq!(negotiate(&a, &b).unwrap(), (2, vec![INPUT.into()]));
    }
    #[test]
    fn disjoint_or_invalid_ranges_rejected() {
        let a = hello(2, 3, "1.0.0", &[]);
        for b in [
            hello(1, 1, "1.0.0", &[]),
            hello(3, 2, "1.0.0", &[]),
            hello(0, 3, "1.0.0", &[]),
        ] {
            assert!(negotiate(&a, &b).unwrap_err().is::<EngineError>());
        }
    }
    #[test]
    fn additive_envelope_and_bounded_capabilities() {
        let remote: Hello = serde_json::from_str(r#"{"min_protocol":1,"max_protocol":1,"app_version":"1.0","capabilities":[],"future":true}"#).unwrap();
        assert!(negotiate(&remote, &remote).is_ok());
        let huge = hello(1, 1, "1", &[&"x".repeat(65)]);
        assert!(negotiate(&remote, &huge).is_err());
    }
}
