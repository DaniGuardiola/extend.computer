//! Translate classified engine failures without inspecting diagnostic wording.
use extend_computer_agent::error::EngineError;

pub(super) use extend_computer_agent::error::is_connection_failure as is_connection_error;

#[derive(Debug)]
pub(super) struct KeychainAccessError;
impl std::fmt::Display for KeychainAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("extend.computer couldn’t access its saved device key. Try again and choose Allow when macOS asks.")
    }
}
impl std::error::Error for KeychainAccessError {}

#[derive(Debug)]
pub(super) struct PairingStartError;
impl std::fmt::Display for PairingStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Couldn’t start pairing. On the other device, open extend.computer and click Pair device. Keep that dialog open, then select the device here again.")
    }
}
impl std::error::Error for PairingStartError {}

pub(crate) fn friendly_error(error: &anyhow::Error) -> String {
    if error.is::<extend_computer_agent::low_jitter::PermissionRequired>() {
        return extend_computer_agent::low_jitter::PermissionRequired.to_string();
    }
    if error.downcast_ref::<PairingStartError>().is_some() {
        return PairingStartError.to_string();
    }
    if error.downcast_ref::<KeychainAccessError>().is_some() {
        return KeychainAccessError.to_string();
    }
    if let Some(failure) = error.downcast_ref::<EngineError>() {
        return match failure {
            EngineError::ProtocolIncompatible | EngineError::FeatureUnavailable => return failure.to_string(),
            EngineError::PeerUnpaired => "The other device is no longer paired. Pair again to reconnect.",
            EngineError::PeerIdentityChanged => "This address belongs to a different device. Pair with it before connecting.",
            EngineError::LocalConsentDenied => "Connection declined on this device.",
            EngineError::RemoteConsentDenied => "The other device declined the connection.",
            EngineError::RequestRejected => "The other device could not accept this request. Check that both devices use a compatible extend.computer version.",
            EngineError::PairingFailed => "Pairing failed. Show a new code on the other device and try again.",
            EngineError::HelperUnavailable => "extend.computer could not start its input helper. Check macOS Permissions and the installed extend.computer version.",
            EngineError::AuthenticationFailed => "The encrypted connection could not be verified. Disconnect and try again.",
        }.into();
    }
    if is_connection_error(error) {
        return "The connection ended. Check that extend.computer is open on the other device and try again.".into();
    }
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keychain_diagnostics_stay_out_of_user_message() {
        let error = anyhow::anyhow!("User canceled the operation.")
            .context(KeychainAccessError)
            .context("background operation failed");
        assert_eq!(friendly_error(&error), "extend.computer couldn’t access its saved device key. Try again and choose Allow when macOS asks.");
        assert!(format!("{error:#}").contains("User canceled"));
    }
    #[test]
    fn classification_survives_context_without_matching_text() {
        let denied =
            anyhow::Error::new(EngineError::RemoteConsentDenied).context("unrelated diagnostic");
        assert_eq!(
            friendly_error(&denied),
            "The other device declined the connection."
        );
        let local = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "local file permission denied",
        ));
        assert!(!is_connection_error(&local));
        assert_eq!(friendly_error(&local), "local file permission denied");
        let helper = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "child pipe closed",
        ))
        .context(EngineError::HelperUnavailable);
        assert!(!is_connection_error(&helper));
        assert!(friendly_error(&helper).starts_with("extend.computer could not start"));
    }
}
