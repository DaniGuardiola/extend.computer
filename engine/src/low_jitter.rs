//! Platform-neutral Wi-Fi lease interface and cursor session integration.
use crate::session::CursorSink;
use anyhow::{Context, Result};
use std::net::IpAddr;

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
