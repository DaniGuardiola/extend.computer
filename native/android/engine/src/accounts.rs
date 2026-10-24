//! Verified account membership is ephemeral and never changes manual trust.
use super::*;
use extend_computer_agent::account_trust::AccountTrust;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Membership {
    pub trust: AccountTrust,
    names: Mutex<BTreeMap<String, String>>,
    pub(super) routes: Mutex<Routes>,
}
#[derive(Default)]
pub(super) struct Routes {
    pub scope: String,
    pub until: Option<std::time::Instant>,
    pub devices: BTreeMap<String, String>,
}
impl Routes {
    pub fn fresh(&self) -> bool {
        self.until
            .is_some_and(|until| until > std::time::Instant::now())
    }
}
struct Member {
    name: String,
    device: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Device {
    id: String,
    fingerprint: String,
    name: String,
    platform: String,
    key_verified: bool,
    online: bool,
}
fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn directory(raw: &str, own: &str) -> Result<BTreeMap<String, Member>> {
    ensure!(raw.len() <= 256 * 1024, "Account directory is too large");
    let devices: Vec<Device> = serde_json::from_str(raw)?;
    ensure!(devices.len() <= 100, "Account directory is too large");
    let mut ids = BTreeSet::new();
    let mut fingerprints = BTreeSet::new();
    let mut names = BTreeMap::new();
    for device in devices {
        ensure!(
            hex(&device.id) && hex(&device.fingerprint),
            "Invalid account device"
        );
        ensure!(
            ids.insert(device.id.clone()) && fingerprints.insert(device.fingerprint.clone()),
            "Duplicate account device"
        );
        extend_computer_agent::session::validate_device_name(&device.name)?;
        ensure!(
            !device.platform.is_empty()
                && device.platform.len() <= 32
                && !device.platform.chars().any(char::is_control),
            "Invalid account platform"
        );
        let _recent_activity = device.online; // Heartbeat is never route reachability.
        if device.key_verified && device.fingerprint != own {
            names.insert(
                device.fingerprint,
                Member {
                    name: device.name,
                    device: device.id,
                },
            );
        }
    }
    Ok(names)
}
pub(super) fn cancel_retired(runtime: &Runtime, previous: BTreeMap<String, String>) -> Result<()> {
    for peer in previous.keys() {
        if runtime.trust.peer(peer)?.is_some() {
            continue;
        }
        if let Some(server) = MEDIA.lock().unwrap().as_ref() {
            let snapshot = server.snapshot();
            if snapshot["peer"].as_str() == Some(peer) {
                if let Some(id) = snapshot["id"].as_str() {
                    server.cancel(Some(id));
                }
            }
        }
        if let Some(sender) = SENDER.lock().unwrap().as_ref() {
            let snapshot = sender.snapshot();
            if snapshot["peer"].as_str() == Some(peer) {
                if let Some(id) = snapshot["id"].as_str() {
                    sender.cancel(id);
                }
            }
        }
        if let Some(sender) = CONTROL.lock().unwrap().as_ref() {
            let snapshot = sender.snapshot();
            if snapshot["peer"].as_str() == Some(peer) {
                if let Some(id) = snapshot["id"].as_str() {
                    sender.cancel(id);
                }
            }
        }
        for slot in [&RECEIVING, &INCOMING] {
            if let Some(listener) = slot.lock().unwrap().as_ref() {
                listener.cancel_peer(peer);
            }
        }
    }
    Ok(())
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_replaceAccountPeers(
    env: JNIEnv,
    _: JClass,
    scope: JString,
    raw: JString,
) -> jstring {
    boundary(env, |env| {
        let scope: String = env.get_string(&scope)?.into();
        ensure!(
            !scope.is_empty() && scope.len() <= 2048 && !scope.chars().any(char::is_control),
            "Invalid account scope"
        );
        let raw: String = env.get_string(&raw)?.into();
        let runtime = RUNTIME.get().context("Engine is not initialized")?;
        let members = directory(&raw, &runtime.identity.fingerprint())?;
        let names = members
            .iter()
            .map(|(fp, m)| (fp.clone(), m.name.clone()))
            .collect::<BTreeMap<_, _>>();
        let devices = members
            .into_iter()
            .map(|(fp, m)| (fp, m.device))
            .collect::<BTreeMap<_, _>>();
        {
            let mut routes = runtime.account.routes.lock().unwrap();
            if routes.scope != scope {
                runtime.relay.configure(None)?;
            }
            for (peer, old_id) in &routes.devices {
                if devices.get(peer) != Some(old_id) {
                    runtime.relay.cancel_peer(peer);
                }
            }
            *routes = Routes {
                scope: scope.clone(),
                until: Some(std::time::Instant::now() + Duration::from_secs(90)),
                devices,
            };
        }
        let previous =
            std::mem::replace(&mut *runtime.account.names.lock().unwrap(), names.clone());
        runtime.account.trust.replace(
            &scope,
            &names.keys().cloned().collect::<Vec<_>>(),
            Duration::from_secs(90),
        );
        cancel_retired(runtime, previous)?;
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_clearAccountPeers(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        if let Some(runtime) = RUNTIME.get() {
            runtime.relay.configure(None)?;
            *runtime.account.routes.lock().unwrap() = Routes::default();
            let previous = std::mem::take(&mut *runtime.account.names.lock().unwrap());
            runtime.account.trust.clear();
            cancel_retired(runtime, previous)?;
        }
        Ok("{}".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_computer_extend_android_NativeEngine_accountPeerNames(
    env: JNIEnv,
    _: JClass,
) -> jstring {
    boundary(env, |_| {
        let Some(runtime) = RUNTIME.get() else {
            return Ok("{}".into());
        };
        let names = runtime.account.names.lock().unwrap();
        let mut available = BTreeMap::new();
        for (peer, name) in names.iter() {
            if runtime.trust.peer(peer)?.is_some() && runtime.trust.is_account_peer(peer)? {
                available.insert(peer, name);
            }
        }
        Ok(serde_json::to_string(&available)?)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_target_is_verified_device_id_not_fingerprint() {
        let peer = "2".repeat(64);
        let id = "3".repeat(64);
        let rows = serde_json::json!([{"id":id,"fingerprint":peer,"name":"Peer","platform":"android","key_verified":true,"online":false}]);
        let map = directory(&rows.to_string(), &"1".repeat(64)).unwrap();
        assert_eq!(map[&peer].device, id);
        assert_eq!(map[&peer].name, "Peer");
        let mut rejected = rows;
        rejected[0]["key_verified"] = serde_json::json!(false);
        assert!(directory(&rejected.to_string(), &"1".repeat(64))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn directory_grants_only_verified_other_devices_and_rejects_partial_or_duplicate_data() {
        let own = "1".repeat(64);
        let peer = "2".repeat(64);
        let row = |fingerprint: &str, verified| serde_json::json!({"id":fingerprint,"fingerprint":fingerprint,"name":"Account peer","platform":"android","key_verified":verified,"online":true});
        assert_eq!(
            directory(
                &serde_json::json!([row(&own, true), row(&peer, true)]).to_string(),
                &own
            )
            .unwrap()
            .len(),
            1
        );
        assert!(
            directory(&serde_json::json!([row(&peer, false)]).to_string(), &own)
                .unwrap()
                .is_empty()
        );
        assert!(directory(
            &serde_json::json!([row(&peer, true), row(&peer, true)]).to_string(),
            &own
        )
        .is_err());
        let mut malformed = row(&peer, true);
        malformed["key_verified"] = serde_json::json!("true");
        assert!(directory(&serde_json::json!([malformed]).to_string(), &own).is_err());
        assert!(directory(&serde_json::json!([{"fingerprint":peer}]).to_string(), &own).is_err());
    }
}
