//! Failures callers may classify without inspecting diagnostic text.
use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    PairingFailed,
    PeerIdentityChanged,
    PeerUnpaired,
    LocalConsentDenied,
    RemoteConsentDenied,
    RequestRejected,
    HelperUnavailable,
    AuthenticationFailed,
}
impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PairingFailed => "pairing failed",
            Self::PeerUnpaired => "peer no longer recognizes this pairing",
            Self::PeerIdentityChanged => "peer identity changed",
            Self::LocalConsentDenied => "permission denied",
            Self::RemoteConsentDenied => "peer denied permission",
            Self::RequestRejected => "request rejected or incompatible peer",
            Self::HelperUnavailable => {
                "native helper unavailable or outdated; check permissions and rebuild both helpers"
            }
            Self::AuthenticationFailed => "encrypted message authentication failed",
        })
    }
}
impl Error for EngineError {}

/// Socket failures may be retried; classified policy, crypto and helper errors may not.
pub fn is_connection_failure(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<EngineError>().is_some()
        || error.is::<crate::low_jitter::PermissionRequired>()
    {
        return false;
    }
    if error
        .downcast_ref::<crate::wire::TransportFailure>()
        .is_some()
    {
        return true;
    }
    use std::io::ErrorKind::*;
    error.downcast_ref::<std::io::Error>().is_some_and(|e| {
        matches!(
            e.kind(),
            ConnectionRefused
                | ConnectionReset
                | ConnectionAborted
                | NotConnected
                | BrokenPipe
                | TimedOut
                | WouldBlock
                | UnexpectedEof
                | Interrupted
        )
    })
}
