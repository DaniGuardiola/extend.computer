//! `viewer`: shows a peer's screen full screen on the primary monitor and,
//! once the runtime grants it, sends the user's input in the window as kind 4
//! packets. The macOS viewer (native/macos/display/Viewer.swift) is the
//! reference; this one is H.264 only, with the pointer drawn into the video.
#![cfg_attr(not(windows), allow(dead_code))]
#[cfg(windows)]
pub mod decode;
pub mod fit;
pub mod input;
pub mod output;
#[cfg(windows)]
mod present;
pub mod protocol;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use window::run;
