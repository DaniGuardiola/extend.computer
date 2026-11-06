//! Android JNI shims for `computer.extend.android.NativeEngine`. Each export only
//! converts Java arguments and reports errors; behavior lives in `api`.
use crate::{api, device_info};
use anyhow::{ensure, Result};
use jni::{
    objects::{JByteArray, JClass, JObject, JString},
    sys::{jboolean, jbyteArray, jdouble, jint, jlong, jstring},
    JNIEnv,
};
use zeroize::Zeroizing;

pub(crate) fn boundary(
    mut env: JNIEnv,
    action: impl FnOnce(&mut JNIEnv) -> Result<String>,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&mut env)));
    let value = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", error.to_string());
            return std::ptr::null_mut();
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", "Native engine failed");
            return std::ptr::null_mut();
        }
    };
    match env.new_string(value) {
        Ok(value) => value.into_raw(),
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "Native engine result unavailable",
            );
            std::ptr::null_mut()
        }
    }
}

/// Byte-array results: `None` maps to Java `null`; errors throw with `failure`
/// as the message for panics.
fn bytes_boundary(
    mut env: JNIEnv,
    failure: &str,
    action: impl FnOnce(&mut JNIEnv) -> Result<Option<Vec<u8>>>,
) -> jbyteArray {
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<jbyteArray> {
            Ok(match action(&mut env)? {
                Some(bytes) => env.byte_array_from_slice(&bytes)?.into_raw(),
                None => std::ptr::null_mut(),
            })
        }));
    match result {
        Ok(Ok(array)) => array,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
            std::ptr::null_mut()
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", failure);
            std::ptr::null_mut()
        }
    }
}

fn text(env: &mut JNIEnv, value: &JString) -> Result<String> {
    Ok(env.get_string(value)?.into())
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_resolvePresence(
    env: JNIEnv,
    _: JClass,
    address: JString,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let address = text(env, &address)?;
        let peer = text(env, &peer)?;
        api::resolve_presence(&address, &peer)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_peerHosts(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::peer_hosts())
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_attachUsb(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| api::attach_usb(&text(env, &peer)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_usbState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::usb_state())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_resolveUsbPresence(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let peer = text(env, &peer)?;
        let id = text(env, &id)?;
        api::resolve_usb_presence(&peer, &id)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_probeUsbPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    id: JString,
) -> jstring {
    boundary(env, |env| {
        let peer = text(env, &peer)?;
        let id = text(env, &id)?;
        api::probe_usb_peer(&peer, &id)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopUsb(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::stop_usb(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_receiveUsb(
    mut env: JNIEnv,
    _: JClass,
    id: JString,
    bytes: JByteArray,
    offset: jint,
    length: jint,
) -> jint {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<usize> {
        ensure!(
            offset >= 0
                && length >= 0
                && length as usize <= api::USB_READ_LIMIT
                && offset
                    .checked_add(length)
                    .is_some_and(|end| end <= env.get_array_length(&bytes).unwrap_or(-1)),
            "Invalid USB read buffer"
        );
        let id = text(&mut env, &id)?;
        let mut buffer = vec![0_i8; length as usize];
        env.get_byte_array_region(&bytes, offset, &mut buffer)?;
        let bytes: Vec<u8> = buffer.into_iter().map(|b| b as u8).collect();
        api::receive_usb(&id, &bytes)
    }));
    match result {
        Ok(Ok(count)) => count as jint,
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
            -1
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/IllegalStateException", "USB engine failed");
            -1
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_nextUsbFrame(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jbyteArray {
    bytes_boundary(env, "USB engine failed", |env| {
        api::next_usb_frame(&text(env, &id)?)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_initialize(
    env: JNIEnv,
    _: JClass,
    seed: JByteArray,
    root: JString,
    context: JObject,
) -> jstring {
    boundary(env, |env| {
        let root = text(env, &root)?;
        let seed = Zeroizing::new(env.convert_byte_array(&seed)?);
        api::initialize(seed, &root, || device_info::detect(env, &context))
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_accountProof(
    env: JNIEnv,
    _: JClass,
    server_key: JString,
    challenge: JString,
) -> jstring {
    boundary(env, |env| {
        let key = text(env, &server_key)?;
        let challenge = text(env, &challenge)?;
        api::account_proof(&key, &challenge)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_replaceAccountPeers(
    env: JNIEnv,
    _: JClass,
    scope: JString,
    raw: JString,
) -> jstring {
    boundary(env, |env| {
        let scope = text(env, &scope)?;
        let raw = text(env, &raw)?;
        api::replace_account_peers(&scope, &raw)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_clearAccountPeers(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::clear_account_peers())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_accountPeerNames(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::account_peer_names())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureAccountRelay(
    env: JNIEnv,
    _: JClass,
    server: JString,
    scope: JString,
    device: JString,
    token: JString,
    fingerprint: JString,
) -> jstring {
    boundary(env, |env| {
        let server = text(env, &server)?;
        let scope = text(env, &scope)?;
        let device = text(env, &device)?;
        let token = Zeroizing::new(text(env, &token)?);
        let fingerprint = text(env, &fingerprint)?;
        api::configure_account_relay(&server, &scope, &device, &token, &fingerprint)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_resolveAccountPresence(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    refresh: jboolean,
) -> jstring {
    boundary(env, |env| {
        api::resolve_account_presence(&text(env, &peer)?, refresh != 0)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_probeAccountPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| api::probe_account_peer(&text(env, &peer)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pairedPeers(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::paired_peers())
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pairCode(
    env: JNIEnv,
    _: JClass,
    address: JString,
    code: JString,
) -> jstring {
    boundary(env, |env| {
        let address = text(env, &address)?;
        let code = Zeroizing::new(text(env, &code)?);
        api::pair_code(&address, &code)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startManualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
    address: JString,
    code: JString,
    name: JString,
) -> jstring {
    boundary(env, |env| {
        let id = text(env, &id)?;
        let address = text(env, &address)?;
        let code = Zeroizing::new(text(env, &code)?);
        let name = text(env, &name)?;
        api::start_manual_pairing(&id, &address, &code, &name)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_manualPairingState(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::manual_pairing_state(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopManualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::stop_manual_pairing(&text(env, &id)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startVisualPairing(
    env: JNIEnv,
    _: JClass,
    address: JString,
    name: JString,
    local_name: JString,
) -> jstring {
    boundary(env, |env| {
        let address = text(env, &address)?;
        let name = text(env, &name)?;
        let local_name = text(env, &local_name)?;
        api::start_visual_pairing(&address, &name, &local_name)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_visualPairingState(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::visual_pairing_state(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_respondVisualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
    request: JString,
    allow: jboolean,
) -> jstring {
    boundary(env, |env| {
        let id = text(env, &id)?;
        let request = text(env, &request)?;
        api::respond_visual_pairing(&id, &request, allow != 0)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopVisualPairing(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::stop_visual_pairing(&text(env, &id)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_probePeer(
    env: JNIEnv,
    _: JClass,
    address: JString,
    peer: JString,
) -> jstring {
    boundary(env, |env| {
        let address = text(env, &address)?;
        let peer = text(env, &peer)?;
        api::probe_peer(&address, &peer)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_forgetPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| api::forget_peer(&text(env, &peer)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_notifyUnpair(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    address: JString,
    operation: JString,
) -> jstring {
    boundary(env, |env| {
        let peer = text(env, &peer)?;
        let address = text(env, &address)?;
        let operation = text(env, &operation)?;
        api::notify_unpair(&peer, &address, &operation)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_updateLocalDeviceName(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| api::update_local_device_name(&text(env, &name)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_removedPeers(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::removed_peers())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_dismissRemovedPeer(
    env: JNIEnv,
    _: JClass,
    peer: JString,
) -> jstring {
    boundary(env, |env| api::dismiss_removed_peer(&text(env, &peer)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startIncoming(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| api::start_incoming(&text(env, &name)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_peerNames(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::peer_names())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startReceiving(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| api::start_receiving(&text(env, &name)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_receivingState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::receiving_state())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopReceiving(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::stop_receiving())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_incomingState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::incoming_state())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_respondIncoming(
    env: JNIEnv,
    _: JClass,
    id: JString,
    allow: jboolean,
) -> jstring {
    boundary(env, |env| {
        api::respond_incoming(&text(env, &id)?, allow != 0)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopIncoming(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::stop_incoming())
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startPresence(
    env: JNIEnv,
    _: JClass,
    name: JString,
) -> jstring {
    boundary(env, |env| api::start_presence(&text(env, &name)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopPresence(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::stop_presence())
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startDisplayReceiver(
    env: JNIEnv,
    _: JClass,
    capabilities: JString,
) -> jstring {
    boundary(env, |env| {
        api::start_display_receiver(&text(env, &capabilities)?)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopDisplayReceiver(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::stop_display_receiver())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::display_state())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayReceiverSettings(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::display_receiver_settings(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_requestDisplayReceiverSettings(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jlong,
    settings: JString,
) -> jstring {
    boundary(env, |env| {
        let id = text(env, &id)?;
        let settings = text(env, &settings)?;
        api::request_display_receiver_settings(&id, revision, &settings)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_attachDisplay(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::attach_display(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayInputState(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::display_input_state(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_setDisplayInputEnabled(
    env: JNIEnv,
    _: JClass,
    id: JString,
    enabled: jboolean,
) -> jstring {
    boundary(env, |env| {
        api::set_display_input_enabled(&text(env, &id)?, enabled != 0)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayInputCommand(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jlong,
    command: JString,
) -> jstring {
    boundary(env, |env| {
        let id = text(env, &id)?;
        let command = text(env, &command)?;
        api::display_input_command(&id, revision, &command)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayConfigured(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jlong,
) -> jstring {
    boundary(env, |env| {
        api::display_configured(&text(env, &id)?, revision)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displayRendered(
    env: JNIEnv,
    _: JClass,
    id: JString,
    revision: jlong,
    presentation_us: jlong,
    device_ns: jlong,
    callback_device_ns: jlong,
) -> jstring {
    boundary(env, |env| {
        api::display_rendered(
            &text(env, &id)?,
            revision,
            presentation_us,
            device_ns,
            callback_device_ns,
        )
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_cancelDisplay(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::cancel_display(&text(env, &id)?))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_nextDisplayFrame(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jbyteArray {
    bytes_boundary(env, "Display engine failed", |env| {
        api::next_display_frame(&text(env, &id)?)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_nextDisplayCursor(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jbyteArray {
    bytes_boundary(env, "Display cursor engine failed", |env| {
        api::next_display_cursor(&text(env, &id)?)
    })
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startControl(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    address: JString,
) -> jstring {
    boundary(env, |env| {
        let peer = text(env, &peer)?;
        let address = text(env, &address)?;
        api::start_control(&peer, &address)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_controlState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::control_state())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_controlEvent(
    env: JNIEnv,
    _: JClass,
    id: JString,
    event: JString,
) -> jstring {
    boundary(env, |env| {
        let id = text(env, &id)?;
        let event = text(env, &event)?;
        api::control_event(&id, &event)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopControl(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::stop_control(&text(env, &id)?))
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureTextReceiver(
    env: JNIEnv,
    _: JClass,
    enabled: jboolean,
) -> jstring {
    boundary(env, |_| api::configure_text_receiver(enabled != 0))
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureInputKeyboardAvailable(
    env: JNIEnv,
    _: JClass,
    enabled: jboolean,
) -> jstring {
    boundary(env, |_| {
        api::configure_input_keyboard_available(enabled != 0)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureInputReceiver(
    env: JNIEnv,
    _: JClass,
    enabled: jboolean,
    width: jdouble,
    height: jdouble,
) -> jstring {
    boundary(env, |_| {
        api::configure_input_receiver(enabled != 0, width, height)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pollInputReceiver(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::poll_input_receiver())
}

#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_startDisplaySender(
    env: JNIEnv,
    _: JClass,
    peer: JString,
    address: JString,
    config: JString,
) -> jstring {
    boundary(env, |env| {
        let peer = text(env, &peer)?;
        let address = text(env, &address)?;
        let config = text(env, &config)?;
        api::start_display_sender(&peer, &address, &config)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_configureDisplaySender(
    env: JNIEnv,
    _: JClass,
    id: JString,
    config: JString,
) -> jstring {
    boundary(env, |env| {
        let id = text(env, &id)?;
        let config = text(env, &config)?;
        api::configure_display_sender(&id, &config)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_displaySenderState(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| api::display_sender_state())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_pushDisplayFrame(
    env: JNIEnv,
    _: JClass,
    id: JString,
    bytes: JByteArray,
    timestamp: jlong,
    keyframe: jboolean,
) -> jstring {
    boundary(env, |env| {
        // Reject before copying an oversized Java array into native memory.
        let length = env.get_array_length(&bytes)?;
        api::check_display_frame(timestamp, usize::try_from(length).unwrap_or(usize::MAX))?;
        let id = text(env, &id)?;
        let bytes = env.convert_byte_array(&bytes)?;
        api::push_display_frame(&id, bytes, timestamp, keyframe != 0)
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_stopDisplaySender(
    env: JNIEnv,
    _: JClass,
    id: JString,
) -> jstring {
    boundary(env, |env| api::stop_display_sender(&text(env, &id)?))
}
