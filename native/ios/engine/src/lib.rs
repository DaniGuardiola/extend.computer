//! iOS C ABI over the shared mobile engine (`extend_android_engine::api`).
//!
//! Conventions, shared by every `ext_*` function:
//! - String arguments are NUL-terminated UTF-8 `const char *`; NULL is an error.
//! - String results are heap JSON objects, either `{"ok":<value>}` or
//!   `{"error":"<message>"}`, and must be released with `ext_string_free`.
//!   `<value>` is the same JSON the Android `NativeEngine` method returns
//!   (a JSON string for `ext_pair_code` and `ext_account_proof`).
//! - Byte results are `ExtBuffer`s released with `ext_buffer_free`. A NULL
//!   `ptr` means "nothing queued" or an error; on error `*error` (when the
//!   out-pointer is non-NULL) receives a plain heap message string to release
//!   with `ext_string_free`, otherwise it is set to NULL.
//! - Byte arguments are `(ptr, len)` pairs; `ptr` may be NULL only when `len` is 0.
//! - Every call catches panics; no Rust panic unwinds into Swift.
#![allow(clippy::missing_safety_doc)]
use anyhow::{bail, Context, Result};
use extend_android_engine::api;
use std::{
    ffi::{c_char, CStr, CString},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr,
};
use zeroize::Zeroizing;

/// Heap bytes owned by the caller until passed to `ext_buffer_free`.
#[repr(C)]
pub struct ExtBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

impl ExtBuffer {
    const EMPTY: ExtBuffer = ExtBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
}

unsafe fn text<'a>(value: *const c_char, name: &str) -> Result<&'a str> {
    if value.is_null() {
        bail!("Missing {name}");
    }
    CStr::from_ptr(value)
        .to_str()
        .with_context(|| format!("Invalid {name}"))
}

unsafe fn bytes<'a>(value: *const u8, len: usize, name: &str) -> Result<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    if value.is_null() {
        bail!("Missing {name}");
    }
    Ok(std::slice::from_raw_parts(value, len))
}

fn c_string(value: String) -> *mut c_char {
    CString::new(value)
        .unwrap_or_else(|_| c"{\"error\":\"Native engine result unavailable\"}".to_owned())
        .into_raw()
}

/// Wraps a JSON value (already serialized) as `{"ok":...}` or an error.
fn respond(action: impl FnOnce() -> Result<String>) -> *mut c_char {
    let body = match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(value)) => format!("{{\"ok\":{value}}}"),
        Ok(Err(error)) => serde_json::json!({ "error": error.to_string() }).to_string(),
        Err(_) => serde_json::json!({ "error": "Native engine failed" }).to_string(),
    };
    c_string(body)
}

/// Encodes a plain string result as a JSON string value.
fn quoted(value: String) -> Result<String> {
    Ok(serde_json::to_string(&value)?)
}

unsafe fn buffer(
    error: *mut *mut c_char,
    failure: &str,
    action: impl FnOnce() -> Result<Option<Vec<u8>>>,
) -> ExtBuffer {
    let result = catch_unwind(AssertUnwindSafe(action));
    let message = match result {
        Ok(Ok(Some(bytes))) => {
            let bytes = bytes.into_boxed_slice();
            let len = bytes.len();
            let out = ExtBuffer {
                ptr: Box::into_raw(bytes).cast::<u8>(),
                len,
            };
            if !error.is_null() {
                *error = ptr::null_mut();
            }
            return out;
        }
        Ok(Ok(None)) => None,
        Ok(Err(error)) => Some(format!("{error:#}")),
        Err(_) => Some(failure.to_owned()),
    };
    if !error.is_null() {
        *error = message.map_or(ptr::null_mut(), c_string);
    }
    ExtBuffer::EMPTY
}

/// Releases a string returned by any `ext_*` function. NULL is ignored.
#[no_mangle]
pub unsafe extern "C" fn ext_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(CString::from_raw(value));
    }
}

/// Releases a buffer returned by any `ext_*` function. A NULL `ptr` is ignored.
#[no_mangle]
pub unsafe extern "C" fn ext_buffer_free(buffer: ExtBuffer) {
    if !buffer.ptr.is_null() {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(
            buffer.ptr, buffer.len,
        )));
    }
}

/// Starts the engine once per process. `device_kind` is "phone", "tablet" or
/// "unknown". Returns `{"ok":{"fingerprint":..,"public_key":..}}`.
#[no_mangle]
pub unsafe extern "C" fn ext_initialize(
    seed: *const u8,
    seed_len: usize,
    root: *const c_char,
    device_kind: *const c_char,
) -> *mut c_char {
    respond(|| {
        let seed = Zeroizing::new(bytes(seed, seed_len, "identity seed")?.to_vec());
        let root = text(root, "storage root")?;
        let kind: api::DeviceKind =
            serde_json::from_value(text(device_kind, "device kind")?.into())?;
        api::initialize(seed, root, || kind)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_account_proof(
    server_key: *const c_char,
    challenge: *const c_char,
) -> *mut c_char {
    respond(|| {
        quoted(api::account_proof(
            text(server_key, "server key")?,
            text(challenge, "challenge")?,
        )?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_resolve_presence(
    address: *const c_char,
    peer: *const c_char,
) -> *mut c_char {
    respond(|| api::resolve_presence(text(address, "address")?, text(peer, "peer")?))
}

#[no_mangle]
pub extern "C" fn ext_peer_hosts() -> *mut c_char {
    respond(api::peer_hosts)
}

#[no_mangle]
pub unsafe extern "C" fn ext_attach_usb(peer: *const c_char) -> *mut c_char {
    respond(|| api::attach_usb(text(peer, "peer")?))
}

#[no_mangle]
pub extern "C" fn ext_usb_state() -> *mut c_char {
    respond(api::usb_state)
}

#[no_mangle]
pub unsafe extern "C" fn ext_resolve_usb_presence(
    peer: *const c_char,
    id: *const c_char,
) -> *mut c_char {
    respond(|| api::resolve_usb_presence(text(peer, "peer")?, text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_probe_usb_peer(peer: *const c_char, id: *const c_char) -> *mut c_char {
    respond(|| api::probe_usb_peer(text(peer, "peer")?, text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_stop_usb(id: *const c_char) -> *mut c_char {
    respond(|| api::stop_usb(text(id, "id")?))
}

/// Returns `{"ok":<accepted byte count>}`.
#[no_mangle]
pub unsafe extern "C" fn ext_receive_usb(
    id: *const c_char,
    data: *const u8,
    len: usize,
) -> *mut c_char {
    respond(|| {
        let id = text(id, "id")?;
        let data = bytes(data, len, "USB bytes")?;
        Ok(api::receive_usb(id, data)?.to_string())
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_next_usb_frame(
    id: *const c_char,
    error: *mut *mut c_char,
) -> ExtBuffer {
    buffer(error, "USB engine failed", || {
        api::next_usb_frame(text(id, "id")?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_replace_account_peers(
    scope: *const c_char,
    directory: *const c_char,
) -> *mut c_char {
    respond(|| api::replace_account_peers(text(scope, "scope")?, text(directory, "directory")?))
}

#[no_mangle]
pub extern "C" fn ext_clear_account_peers() -> *mut c_char {
    respond(api::clear_account_peers)
}

#[no_mangle]
pub extern "C" fn ext_account_peer_names() -> *mut c_char {
    respond(api::account_peer_names)
}

#[no_mangle]
pub unsafe extern "C" fn ext_configure_account_relay(
    server: *const c_char,
    scope: *const c_char,
    device: *const c_char,
    token: *const c_char,
    fingerprint: *const c_char,
) -> *mut c_char {
    respond(|| {
        api::configure_account_relay(
            text(server, "server")?,
            text(scope, "scope")?,
            text(device, "device")?,
            text(token, "token")?,
            text(fingerprint, "fingerprint")?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_resolve_account_presence(
    peer: *const c_char,
    refresh: bool,
) -> *mut c_char {
    respond(|| api::resolve_account_presence(text(peer, "peer")?, refresh))
}

#[no_mangle]
pub unsafe extern "C" fn ext_probe_account_peer(peer: *const c_char) -> *mut c_char {
    respond(|| api::probe_account_peer(text(peer, "peer")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_update_local_device_name(name: *const c_char) -> *mut c_char {
    respond(|| api::update_local_device_name(text(name, "name")?))
}

#[no_mangle]
pub extern "C" fn ext_paired_peers() -> *mut c_char {
    respond(api::paired_peers)
}

/// Returns `{"ok":"<peer fingerprint>"}`.
#[no_mangle]
pub unsafe extern "C" fn ext_pair_code(address: *const c_char, code: *const c_char) -> *mut c_char {
    respond(|| {
        quoted(api::pair_code(
            text(address, "address")?,
            text(code, "code")?,
        )?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_manual_pairing(
    id: *const c_char,
    address: *const c_char,
    code: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    respond(|| {
        api::start_manual_pairing(
            text(id, "id")?,
            text(address, "address")?,
            text(code, "code")?,
            text(name, "name")?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_manual_pairing_state(id: *const c_char) -> *mut c_char {
    respond(|| api::manual_pairing_state(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_stop_manual_pairing(id: *const c_char) -> *mut c_char {
    respond(|| api::stop_manual_pairing(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_visual_pairing(
    address: *const c_char,
    name: *const c_char,
    local_name: *const c_char,
) -> *mut c_char {
    respond(|| {
        api::start_visual_pairing(
            text(address, "address")?,
            text(name, "name")?,
            text(local_name, "local name")?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_visual_pairing_state(id: *const c_char) -> *mut c_char {
    respond(|| api::visual_pairing_state(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_respond_visual_pairing(
    id: *const c_char,
    request: *const c_char,
    allow: bool,
) -> *mut c_char {
    respond(|| api::respond_visual_pairing(text(id, "id")?, text(request, "request")?, allow))
}

#[no_mangle]
pub unsafe extern "C" fn ext_stop_visual_pairing(id: *const c_char) -> *mut c_char {
    respond(|| api::stop_visual_pairing(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_probe_peer(
    address: *const c_char,
    peer: *const c_char,
) -> *mut c_char {
    respond(|| api::probe_peer(text(address, "address")?, text(peer, "peer")?))
}

/// Returns `{"ok":{"operation":"<id>"}}` for `ext_notify_unpair`.
#[no_mangle]
pub unsafe extern "C" fn ext_forget_peer(peer: *const c_char) -> *mut c_char {
    respond(|| api::forget_peer(text(peer, "peer")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_notify_unpair(
    peer: *const c_char,
    address: *const c_char,
    operation: *const c_char,
) -> *mut c_char {
    respond(|| {
        api::notify_unpair(
            text(peer, "peer")?,
            text(address, "address")?,
            text(operation, "operation")?,
        )
    })
}

#[no_mangle]
pub extern "C" fn ext_removed_peers() -> *mut c_char {
    respond(api::removed_peers)
}

#[no_mangle]
pub unsafe extern "C" fn ext_dismiss_removed_peer(peer: *const c_char) -> *mut c_char {
    respond(|| api::dismiss_removed_peer(text(peer, "peer")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_incoming(name: *const c_char) -> *mut c_char {
    respond(|| api::start_incoming(text(name, "name")?))
}

#[no_mangle]
pub extern "C" fn ext_peer_names() -> *mut c_char {
    respond(api::peer_names)
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_receiving(name: *const c_char) -> *mut c_char {
    respond(|| api::start_receiving(text(name, "name")?))
}

#[no_mangle]
pub extern "C" fn ext_receiving_state() -> *mut c_char {
    respond(api::receiving_state)
}

#[no_mangle]
pub extern "C" fn ext_stop_receiving() -> *mut c_char {
    respond(api::stop_receiving)
}

#[no_mangle]
pub extern "C" fn ext_incoming_state() -> *mut c_char {
    respond(api::incoming_state)
}

#[no_mangle]
pub unsafe extern "C" fn ext_respond_incoming(request: *const c_char, allow: bool) -> *mut c_char {
    respond(|| api::respond_incoming(text(request, "request")?, allow))
}

#[no_mangle]
pub extern "C" fn ext_stop_incoming() -> *mut c_char {
    respond(api::stop_incoming)
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_presence(name: *const c_char) -> *mut c_char {
    respond(|| api::start_presence(text(name, "name")?))
}

#[no_mangle]
pub extern "C" fn ext_stop_presence() -> *mut c_char {
    respond(api::stop_presence)
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_display_receiver(capabilities: *const c_char) -> *mut c_char {
    respond(|| api::start_display_receiver(text(capabilities, "capabilities")?))
}

#[no_mangle]
pub extern "C" fn ext_stop_display_receiver() -> *mut c_char {
    respond(api::stop_display_receiver)
}

#[no_mangle]
pub extern "C" fn ext_display_state() -> *mut c_char {
    respond(api::display_state)
}

#[no_mangle]
pub unsafe extern "C" fn ext_display_receiver_settings(id: *const c_char) -> *mut c_char {
    respond(|| api::display_receiver_settings(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_request_display_receiver_settings(
    id: *const c_char,
    revision: i64,
    settings: *const c_char,
) -> *mut c_char {
    respond(|| {
        api::request_display_receiver_settings(
            text(id, "id")?,
            revision,
            text(settings, "settings")?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_attach_display(id: *const c_char) -> *mut c_char {
    respond(|| api::attach_display(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_display_input_state(id: *const c_char) -> *mut c_char {
    respond(|| api::display_input_state(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_set_display_input_enabled(
    id: *const c_char,
    enabled: bool,
) -> *mut c_char {
    respond(|| api::set_display_input_enabled(text(id, "id")?, enabled))
}

#[no_mangle]
pub unsafe extern "C" fn ext_display_input_command(
    id: *const c_char,
    revision: i64,
    command: *const c_char,
) -> *mut c_char {
    respond(|| api::display_input_command(text(id, "id")?, revision, text(command, "command")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_display_configured(id: *const c_char, revision: i64) -> *mut c_char {
    respond(|| api::display_configured(text(id, "id")?, revision))
}

#[no_mangle]
pub unsafe extern "C" fn ext_display_rendered(
    id: *const c_char,
    revision: i64,
    presentation_us: i64,
    device_ns: i64,
    callback_device_ns: i64,
) -> *mut c_char {
    respond(|| {
        api::display_rendered(
            text(id, "id")?,
            revision,
            presentation_us,
            device_ns,
            callback_device_ns,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_cancel_display(id: *const c_char) -> *mut c_char {
    respond(|| api::cancel_display(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_next_display_frame(
    id: *const c_char,
    error: *mut *mut c_char,
) -> ExtBuffer {
    buffer(error, "Display engine failed", || {
        api::next_display_frame(text(id, "id")?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_next_display_cursor(
    id: *const c_char,
    error: *mut *mut c_char,
) -> ExtBuffer {
    buffer(error, "Display cursor engine failed", || {
        api::next_display_cursor(text(id, "id")?)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_control(
    peer: *const c_char,
    address: *const c_char,
) -> *mut c_char {
    respond(|| api::start_control(text(peer, "peer")?, text(address, "address")?))
}

#[no_mangle]
pub extern "C" fn ext_control_state() -> *mut c_char {
    respond(api::control_state)
}

#[no_mangle]
pub unsafe extern "C" fn ext_control_event(id: *const c_char, event: *const c_char) -> *mut c_char {
    respond(|| api::control_event(text(id, "id")?, text(event, "event")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_stop_control(id: *const c_char) -> *mut c_char {
    respond(|| api::stop_control(text(id, "id")?))
}

#[no_mangle]
pub unsafe extern "C" fn ext_start_display_sender(
    peer: *const c_char,
    address: *const c_char,
    config: *const c_char,
) -> *mut c_char {
    respond(|| {
        api::start_display_sender(
            text(peer, "peer")?,
            text(address, "address")?,
            text(config, "config")?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_configure_display_sender(
    id: *const c_char,
    config: *const c_char,
) -> *mut c_char {
    respond(|| api::configure_display_sender(text(id, "id")?, text(config, "config")?))
}

#[no_mangle]
pub extern "C" fn ext_display_sender_state() -> *mut c_char {
    respond(api::display_sender_state)
}

/// `timestamp_ns` must be positive; frames are limited to 8 MiB.
#[no_mangle]
pub unsafe extern "C" fn ext_push_display_frame(
    id: *const c_char,
    data: *const u8,
    len: usize,
    timestamp_ns: i64,
    keyframe: bool,
) -> *mut c_char {
    respond(|| {
        api::check_display_frame(timestamp_ns, len)?;
        let id = text(id, "id")?;
        let data = bytes(data, len, "frame bytes")?.to_vec();
        api::push_display_frame(id, data, timestamp_ns, keyframe)
    })
}

#[no_mangle]
pub unsafe extern "C" fn ext_stop_display_sender(id: *const c_char) -> *mut c_char {
    respond(|| api::stop_display_sender(text(id, "id")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn take(value: *mut c_char) -> serde_json::Value {
        assert!(!value.is_null());
        let parsed = serde_json::from_str(CStr::from_ptr(value).to_str().unwrap()).unwrap();
        ext_string_free(value);
        parsed
    }

    #[test]
    fn results_are_wrapped_and_arguments_are_checked() {
        unsafe {
            assert_eq!(
                take(ext_display_state()),
                serde_json::json!({"ok":{"state":"idle"}})
            );
            assert_eq!(
                take(ext_stop_usb(ptr::null())),
                serde_json::json!({"error":"Missing id"})
            );
            assert_eq!(
                take(ext_paired_peers()),
                serde_json::json!({"error":"Engine is not initialized"})
            );
            let bad = [0xff_u8, 0];
            assert_eq!(
                take(ext_stop_usb(bad.as_ptr().cast())),
                serde_json::json!({"error":"Invalid id"})
            );
            assert_eq!(
                take(ext_push_display_frame(
                    c"x".as_ptr(),
                    ptr::null(),
                    0,
                    0,
                    false
                )),
                serde_json::json!({"error":"Invalid capture timestamp"})
            );
        }
    }

    #[test]
    fn buffers_report_empty_queue_and_errors() {
        unsafe {
            let mut error: *mut c_char = ptr::null_mut();
            let frame = ext_next_display_frame(c"missing".as_ptr(), &mut error);
            assert!(frame.ptr.is_null());
            assert!(!error.is_null());
            assert_eq!(
                CStr::from_ptr(error).to_str().unwrap(),
                "Receiving disabled"
            );
            ext_string_free(error);
            let frame = ext_next_display_frame(ptr::null(), ptr::null_mut());
            assert!(frame.ptr.is_null());
            ext_buffer_free(frame);
            let owned = buffer(&mut error, "failed", || Ok(Some(vec![1, 2, 3])));
            assert!(error.is_null());
            assert_eq!(std::slice::from_raw_parts(owned.ptr, owned.len), &[1, 2, 3]);
            ext_buffer_free(owned);
        }
    }
}
