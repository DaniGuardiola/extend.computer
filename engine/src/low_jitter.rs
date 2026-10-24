//! Platform-neutral Wi-Fi lease interface and cursor session integration.
use crate::session::CursorSink;
use anyhow::{Context, Result};
use std::net::IpAddr;

#[cfg(target_os = "macos")]
pub use crate::platform::low_jitter::{ready_result, ReadinessError, ReadinessFailure};

pub use crate::platform::low_jitter::{
    configure_app_helper, ready, require_ready_for_peer, required_for_peer, start_for_peer,
    start_required, Lease,
};

#[derive(Debug)]
pub struct PermissionRequired;
impl std::fmt::Display for PermissionRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Allow Wi-Fi optimization in Permissions, then reconnect. It is required for sharing controls over Wi-Fi.")
    }
}
impl std::error::Error for PermissionRequired {}

pub fn maintain_required(lease: &mut Option<Lease>) -> Result<()> {
    if let Some(active) = lease {
        active.maintain().context(PermissionRequired)?;
    }
    Ok(())
}
pub fn maintain(lease: &mut Option<Lease>) {
    if let Err(error) = maintain_required(lease) {
        eprintln!("{error}");
        lease.take();
    }
}
/// Wrap only an explicitly configured cursor sink. A denied request never
/// inspects routes or starts a privileged lease.
pub struct ManagedCursor<S> {
    inner: S,
    peer: IpAddr,
    enabled: bool,
    lease: Option<Lease>,
    required: bool,
}
impl<S> ManagedCursor<S> {
    pub fn required(inner: S, peer: IpAddr) -> Self {
        Self {
            inner,
            peer,
            enabled: true,
            lease: None,
            required: true,
        }
    }
    fn start(&mut self) -> Result<()> {
        self.lease = if self.required {
            start_required(self.peer)?
        } else {
            start_for_peer(self.peer, self.enabled)
        };
        Ok(())
    }
    pub fn new(inner: S, peer: IpAddr, enabled: bool) -> Self {
        Self {
            inner,
            peer,
            enabled,
            lease: None,
            required: false,
        }
    }
}
impl<S: CursorSink> CursorSink for ManagedCursor<S> {
    fn shortcut_policy(&self) -> Option<crate::input::ShortcutPolicy> {
        self.inner.shortcut_policy()
    }
    fn supports_text_input(&self) -> bool {
        self.inner.supports_text_input()
    }
    fn device_name(&self) -> Option<String> {
        self.inner.device_name()
    }
    fn peer_name(&mut self, peer: &str, name: &str) -> Result<()> {
        self.inner.peer_name(peer, name)
    }
    fn unpaired(&mut self, peer: &str) -> Result<()> {
        self.inner.unpaired(peer)
    }
    fn start_control(&mut self, peer: &str) -> Result<bool> {
        if !self.inner.start_control(peer)? {
            return Ok(false);
        }
        self.start()?;
        Ok(true)
    }
    fn start_cursor(&mut self, peer: &str) -> Result<bool> {
        if !self.inner.start_cursor(peer)? {
            return Ok(false);
        }
        self.start()?;
        Ok(true)
    }
    fn start_input(&mut self, peer: &str) -> Result<bool> {
        if !self.inner.start_input(peer)? {
            return Ok(false);
        }
        self.start()?;
        Ok(true)
    }
    fn input(&mut self, event: &crate::input::InputEvent) -> Result<()> {
        self.inner.input(event)
    }
    fn heartbeat(&mut self) -> Result<()> {
        self.inner.heartbeat()?;
        if self.required {
            maintain_required(&mut self.lease)?;
        } else {
            maintain(&mut self.lease);
        }
        Ok(())
    }
    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.inner.move_to(x, y)
    }
    fn display_size(&self) -> Result<crate::session::DisplaySize> {
        self.inner.display_size()
    }
    fn finish(&mut self) {
        // Release held input before waiting for privileged lease cleanup.
        self.inner.finish();
        self.lease.take();
    }
}

#[cfg(all(test, unix))]
mod text_adapter_tests {
    use super::*;
    use crate::{cursor::NativeSink, input::InputEvent};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn managed_native_text_tracks_actual_helper_readiness_and_finish() {
        for supported in [false, true] {
            for required in [false, true] {
                for control in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let path = temp.path().join("helper");
                    std::fs::write(&path, format!(
                        "#!/bin/sh\nprintf '%s\\n' 'READY 1920 1080{}'\nwhile IFS= read -r line; do printf '%s\\n' OK; done\n",
                        if supported { " input-text-v1 input-shortcuts-command-v1" } else { "" },
                    )).unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let inner = NativeSink::new(Some(path));
                    let peer = "127.0.0.1".parse().unwrap();
                    let mut sink = if required {
                        ManagedCursor::required(inner, peer)
                    } else {
                        ManagedCursor::new(inner, peer, false)
                    };
                    assert!(!sink.supports_text_input());
                    assert_eq!(sink.shortcut_policy(), None);
                    // Exercise the actual required decorator and subprocess READY contract
                    // without acquiring a privileged network lease in a host unit test.
                    if required {
                        if control {
                            sink.inner.start_control("peer").unwrap();
                        } else {
                            sink.inner.start_input("peer").unwrap();
                        }
                    } else if control {
                        sink.start_control("peer").unwrap();
                    } else {
                        sink.start_input("peer").unwrap();
                    }
                    assert_eq!(sink.supports_text_input(), supported);
                    assert_eq!(
                        sink.shortcut_policy(),
                        if supported {
                            Some(crate::input::ShortcutPolicy::Command)
                        } else {
                            None
                        }
                    );
                    assert_eq!(
                        sink.input(&InputEvent::Text {
                            value: "e\u{301}👩‍💻中文".into()
                        })
                        .is_ok(),
                        supported
                    );
                    sink.input(&InputEvent::Release).unwrap();
                    sink.finish();
                    assert!(!sink.supports_text_input());
                    assert_eq!(sink.shortcut_policy(), None);
                }
            }
        }
    }
}
