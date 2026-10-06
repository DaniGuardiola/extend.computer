use super::*;
#[track_caller]
fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "GUI state did not settle");
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
#[cfg(unix)]
fn gui_pairing_authorizes_control_connect_disconnect_and_cancel() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let helper = temp.path().join("helper");
    std::fs::write(
        &helper,
        r#"#!/bin/sh
case "$1" in
status) echo 'listen=true post=true wifi=true';;
inject-control) echo 'READY 1728 1117'; while IFS= read -r line; do echo OK; done;;
capture-control-*) echo 'READY 1512 982'; while IFS= read -r line; do echo "$line" >> "$(dirname "$0")/capture-commands"; done;;
*) exit 1;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let receiver = Desktop::new(temp.path().join("receiver"), helper.clone()).unwrap();
    let sender = Desktop::new(temp.path().join("sender"), helper).unwrap();
    *receiver.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
    *sender.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
    let receiver_id = receiver.identity().unwrap().fingerprint();
    let sender_id = sender.identity().unwrap().fingerprint();
    // Failed listener startup must not publish an unusable pairing code.
    let occupied = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    assert!(receiver.receive_at(true, port).is_err());
    assert!(receiver.snapshot().unwrap().code.is_none());
    drop(occupied);
    // Discovery can find a receiver whose pairing dialog is closed.
    receiver.receive_at(false, port).unwrap();
    sender
        .pair_nearby(Device {
            name: "Not ready".into(),
            address: format!("127.0.0.1:{port}"),
            edge: "left".into(),
        })
        .unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none() && receiver.inner.lock().unwrap().job.is_none()
    });
    assert_eq!(sender.snapshot().unwrap().error.as_deref(), Some("Couldn’t start pairing. On the other device, open extend.computer and click Pair device. Keep that dialog open, then select the device here again."));
    assert!(sender.snapshot().unwrap().peers.is_empty());
    assert!(receiver.approvals.current().is_none());
    receiver.receive_at(true, port).unwrap();
    assert!(receiver.snapshot().unwrap().discoverable);
    // An open dialog automatically renews expired windows; closing withdraws presence.
    receiver
        .inner
        .lock()
        .unwrap()
        .code
        .as_mut()
        .unwrap()
        .expires = Instant::now();
    wait(|| receiver.snapshot().unwrap().code_seconds > 0);
    receiver.close_pairing();
    assert!(!receiver.snapshot().unwrap().discoverable);
    assert!(receiver.snapshot().unwrap().code.is_none());
    assert!(receiver.snapshot().unwrap().receiving);
    receiver.receive_at(true, port).unwrap();
    assert!(receiver.snapshot().unwrap().discoverable);
    let code = receiver.snapshot().unwrap().code.unwrap();
    let device = Device {
        name: "Test Mac".into(),
        address: format!("127.0.0.1:{port}"),
        edge: "left".into(),
    };
    sender
        .pair(device.clone(), code.replace('-', "").to_ascii_uppercase())
        .unwrap();
    wait(|| receiver.approvals.current().is_some());
    let request = receiver.approvals.current().unwrap();
    assert_eq!(request.kind, "pair");
    receiver
        .approvals
        .answer(request.id, Answer::Remember)
        .unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(
        sender.snapshot().unwrap().error.is_none(),
        "{:?}",
        sender.snapshot().unwrap().error
    );
    assert!(receiver
        .store()
        .unwrap()
        .peer(&sender_id)
        .unwrap()
        .is_some());
    assert_eq!(sender.snapshot().unwrap().peers[0].name, "Test Mac");
    assert_eq!(
        receiver.snapshot().unwrap().peers[0].name,
        peers::local_name()
    );
    // Backfill an older address-only name, then preserve an explicit user name.
    let mut legacy = receiver.devices.lock().unwrap()[&sender_id].clone();
    legacy.name = format!(
        "Computer · {}",
        legacy.address.parse::<SocketAddr>().unwrap().ip()
    );
    receiver.save_device(&sender_id, legacy.clone()).unwrap();
    sender
        .sync_peer_name(&receiver_id, &device, &sender.identity().unwrap(), None)
        .unwrap();
    assert_eq!(
        receiver.devices.lock().unwrap()[&sender_id].name,
        peers::local_name()
    );
    wait(|| receiver.inner.lock().unwrap().job.is_none());
    legacy.name = "My custom name".into();
    receiver.save_device(&sender_id, legacy).unwrap();
    sender
        .sync_peer_name(&receiver_id, &device, &sender.identity().unwrap(), None)
        .unwrap();
    assert_eq!(
        receiver.devices.lock().unwrap()[&sender_id].name,
        "My custom name"
    );
    wait(|| receiver.inner.lock().unwrap().job.is_none());
    sender.connect(receiver_id.clone(), device.clone()).unwrap();
    wait(|| {
        sender
            .snapshot()
            .unwrap()
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Connected)
    });
    assert!(receiver
        .store()
        .unwrap()
        .peer(&sender_id)
        .unwrap()
        .is_some());
    wait(|| {
        receiver
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|s| s.phase == Phase::Connected)
    });
    assert!(receiver.approvals.current().is_none());
    let session_id = sender.snapshot().unwrap().session.unwrap().id;
    let mut changed = device.clone();
    changed.edge = "right".into();
    sender.save_device(&receiver_id, changed).unwrap();
    wait(|| {
        std::fs::read_to_string(temp.path().join("capture-commands"))
            .is_ok_and(|commands| commands.contains("EDGE right"))
    });
    let session = sender.snapshot().unwrap().session.unwrap();
    assert_eq!(session.id, session_id);
    assert!(session.phase == Phase::Connected);
    assert!(receiver.approvals.current().is_none());
    // A real socket failure reconnects the existing outgoing job using the
    // pinned peer and remembered control grant, without another approval.
    let old_receiver_job = receiver.inner.lock().unwrap().job.as_ref().unwrap().view.id;
    receiver
        .inner
        .lock()
        .unwrap()
        .job
        .as_ref()
        .unwrap()
        .socket
        .as_ref()
        .unwrap()
        .shutdown(Shutdown::Both)
        .unwrap();
    wait(|| {
        receiver
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|s| s.id != old_receiver_job && s.phase == Phase::Connected)
    });
    wait(|| {
        sender
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|s| s.id == session_id && s.phase == Phase::Connected)
    });
    assert!(receiver.approvals.current().is_none());
    // Cancellation during reconnect backoff must prevent a later restart.
    receiver
        .inner
        .lock()
        .unwrap()
        .job
        .as_ref()
        .unwrap()
        .socket
        .as_ref()
        .unwrap()
        .shutdown(Shutdown::Both)
        .unwrap();
    wait(|| {
        sender
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|s| s.phase == Phase::Connecting)
    });
    sender.disconnect();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(sender.snapshot().unwrap().error.is_none());
    // Pairing listener remains usable, but receiving off must not prompt for control.
    receiver.inner.lock().unwrap().receiving_enabled = false;
    sender.connect(receiver_id.clone(), device.clone()).unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(receiver.approvals.current().is_none());
    receiver.receive_at(false, port).unwrap();
    sender.connect(receiver_id, device).unwrap();
    wait(|| {
        sender
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|s| s.phase == Phase::Connected)
    });
    assert!(receiver.approvals.current().is_none());
    sender.disconnect();
    wait(|| {
        receiver.snapshot().unwrap().session.is_none()
            && sender.snapshot().unwrap().session.is_none()
    });
    assert!(receiver.approvals.current().is_none());
    receiver.stop_receiving().unwrap();
    wait(|| receiver.inner.lock().unwrap().listener.is_none());
    // Fresh identities exercise visual pairing independently of previous manual trust.
    *receiver.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
    *sender.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
    let visual_sender = sender.identity().unwrap().fingerprint();
    let visual_receiver = receiver.identity().unwrap().fingerprint();
    let device = Device {
        name: "Nearby Mac".into(),
        address: format!("127.0.0.1:{port}"),
        edge: "left".into(),
    };
    receiver.receive_at(true, port).unwrap();
    sender.pair_nearby(device.clone()).unwrap();
    wait(|| sender.approvals.current().is_some() && receiver.approvals.current().is_some());
    let a = sender.approvals.current().unwrap();
    let b = receiver.approvals.current().unwrap();
    assert_eq!(a.kind, "verify");
    assert_eq!(a.symbols, b.symbols);
    assert_eq!(a.symbols.unwrap().len(), 8);
    sender.approvals.answer(a.id, Answer::Remember).unwrap();
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_none());
    receiver.approvals.answer(b.id, Answer::Deny).unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_none());
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_none());
    receiver.receive_at(true, port).unwrap();
    sender.pair_nearby(device.clone()).unwrap();
    wait(|| sender.approvals.current().is_some() && receiver.approvals.current().is_some());
    sender.close_pairing();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_none());
    receiver.receive_at(true, port).unwrap();
    sender.pair_nearby(device).unwrap();
    wait(|| sender.approvals.current().is_some() && receiver.approvals.current().is_some());
    sender
        .approvals
        .answer(sender.approvals.current().unwrap().id, Answer::Remember)
        .unwrap();
    receiver
        .approvals
        .answer(receiver.approvals.current().unwrap().id, Answer::Remember)
        .unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_some());
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_some());
    assert_eq!(
        receiver.devices.lock().unwrap()[&visual_sender].name,
        peers::local_name()
    );
    receiver
        .update_peer_name(&visual_sender, "Unexpected rename")
        .unwrap();
    assert_eq!(
        receiver.devices.lock().unwrap()[&visual_sender].name,
        peers::local_name()
    );
    sender.unpair(&visual_receiver).unwrap();
    wait(|| {
        receiver
            .store()
            .unwrap()
            .peer(&visual_sender)
            .unwrap()
            .is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_none());
    assert!(receiver
        .devices
        .lock()
        .unwrap()
        .contains_key(&visual_sender));
    assert!(sender.snapshot().unwrap().removed_peers.is_empty());
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_none());
    assert!(!sender
        .devices
        .lock()
        .unwrap()
        .contains_key(&visual_receiver));
    assert!(!sender
        .snapshot()
        .unwrap()
        .peers
        .iter()
        .any(|p| p.id == visual_receiver));
    receiver.receive_at(true, port).unwrap();
    sender
        .pair_nearby(Device {
            name: "Paired again".into(),
            address: format!("127.0.0.1:{port}"),
            edge: "left".into(),
        })
        .unwrap();
    wait(|| sender.approvals.current().is_some() && receiver.approvals.current().is_some());
    assert!(sender.unpair(&visual_receiver).is_err());
    sender
        .approvals
        .answer(sender.approvals.current().unwrap().id, Answer::Remember)
        .unwrap();
    receiver
        .approvals
        .answer(receiver.approvals.current().unwrap().id, Answer::Remember)
        .unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_some());
    assert!(sender.snapshot().unwrap().removed_peers.is_empty());
    assert!(receiver.snapshot().unwrap().removed_peers.is_empty());
    assert!(sender.dismiss_removed(&visual_receiver).is_err());
    // Offline is never evidence of unpairing.
    receiver.stop_receiving().unwrap();
    wait(|| receiver.inner.lock().unwrap().listener.is_none());
    let offline = sender.devices.lock().unwrap()[&visual_receiver].clone();
    assert!(sender
        .check_peer(&visual_receiver, &offline, &sender.identity().unwrap())
        .is_err());
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_some());
    sender.unpair(&visual_receiver).unwrap();
    assert!(sender
        .store()
        .unwrap()
        .peer(&visual_receiver)
        .unwrap()
        .is_none());
    assert!(sender
        .store()
        .unwrap()
        .load()
        .unwrap()
        .pending_unpairs
        .is_empty());
    assert!(sender.snapshot().unwrap().removed_peers.is_empty());
    let reloaded = Desktop::new(sender.root.clone(), sender.helper.clone()).unwrap();
    assert!(reloaded.snapshot().unwrap().removed_peers.is_empty());
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_some());
    // A stale authenticated check cannot remove a pairing after a newer GUI job.
    let epoch = receiver.inner.lock().unwrap().next;
    let (job, _) = receiver.reserve(SessionKind::Pair, None).unwrap();
    receiver.finish(job, Ok(()));
    receiver.apply_peer_unpaired(&visual_sender, epoch).unwrap();
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_some());
    sender.receive_at(false, port).unwrap();
    let target = Device {
        name: "Sender".into(),
        address: format!("127.0.0.1:{port}"),
        edge: "left".into(),
    };
    receiver
        .check_peer(&visual_sender, &target, &receiver.identity().unwrap())
        .unwrap();
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_none());
    assert_eq!(receiver.snapshot().unwrap().removed_peers.len(), 1);
    assert!(receiver.snapshot().unwrap().notification.is_some());
    assert!(receiver.approvals.current().is_none());
    // Explicit connection reconciles the same authenticated response without input approval.
    receiver.store().unwrap().remember(&visual_sender).unwrap();
    receiver
        .save_device(&visual_sender, target.clone())
        .unwrap();
    receiver.connect(visual_sender.clone(), target).unwrap();
    wait(|| receiver.snapshot().unwrap().session.is_none());
    assert!(receiver
        .store()
        .unwrap()
        .peer(&visual_sender)
        .unwrap()
        .is_none());
    assert!(receiver.snapshot().unwrap().error.is_none());
    assert_eq!(receiver.snapshot().unwrap().removed_peers.len(), 1);
    receiver.dismiss_removed(&visual_sender).unwrap();
    assert!(receiver.snapshot().unwrap().removed_peers.is_empty());
    assert!(receiver.snapshot().unwrap().notification.is_some());
    receiver.shutdown();
    sender.shutdown();
    wait(|| sender.inner.lock().unwrap().listener.is_none());
}

#[test]
fn legacy_local_placeholders_are_hidden_but_remote_notices_survive() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("unpaired.json"),
        serde_json::to_vec(&serde_json::json!({
            "local": "Unpaired on this computer. Pair again to reconnect.",
            "remote": "Unpaired from the other computer. Pair again to reconnect."
        }))
        .unwrap(),
    )
    .unwrap();
    let app = Desktop::new(temp.path().into(), temp.path().join("unused-helper")).unwrap();
    let removals = app.removals.lock().unwrap();
    assert!(!removals.contains_key("local"));
    assert_eq!(removals["remote"], unpair::REMOTE_UNPAIR_MESSAGE);
}

#[test]
fn pairing_advertisement_is_withdrawn_when_dialog_closes() {
    use mdns_sd::ServiceEvent;
    let dir = tempfile::tempdir().unwrap();
    let app = Desktop::new(dir.path().into(), dir.path().join("unused-helper")).unwrap();
    *app.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let daemon = extend_computer_agent::discovery::ipv4_daemon().unwrap();
    let events = daemon
        .browse(extend_computer_agent::discovery::PAIRING_SERVICE)
        .unwrap();
    app.receive_at(true, port).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let fullname = loop {
        assert!(
            Instant::now() < deadline,
            "pairing advertisement never resolved"
        );
        if let Ok(ServiceEvent::ServiceResolved(info)) =
            events.recv_timeout(Duration::from_millis(100))
        {
            if info.get_property_val_str("discovery_id") == Some(app.discovery_id.as_str()) {
                break info.get_fullname().to_owned();
            }
        }
    };
    app.close_pairing();
    assert!(!app.snapshot().unwrap().discoverable);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            Instant::now() < deadline,
            "closed dialog advertisement was not withdrawn"
        );
        if let Ok(ServiceEvent::ServiceRemoved(_, name)) =
            events.recv_timeout(Duration::from_millis(100))
        {
            if name == fullname {
                break;
            }
        }
    }
    assert!(!app.snapshot().unwrap().receiving);
    app.shutdown();
    daemon.shutdown().unwrap();
    wait(|| app.inner.lock().unwrap().listener.is_none());
}

#[test]
#[cfg(unix)]
fn receiving_requires_accessibility_and_wifi_but_pairing_does_not() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let helper = temp.path().join("helper");
    let status = temp.path().join("status");
    std::fs::write(&helper, "#!/bin/sh\ncat \"$(dirname \"$0\")/status\"\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&status, "listen=false post=false wifi=true\n").unwrap();
    let app = Desktop::new(temp.path().join("app"), helper).unwrap();
    *app.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    assert!(app.receive_at(false, port).is_err());
    assert!(!app.snapshot().unwrap().receiving);
    assert!(app.inner.lock().unwrap().listener.is_none());
    app.receive_at(true, port).unwrap();
    assert!(app.snapshot().unwrap().discoverable);
    assert!(!app.snapshot().unwrap().receiving);
    app.close_pairing();
    assert!(app.receive_at(false, port).is_err());
    std::fs::write(&status, "listen=false post=true wifi=true\n").unwrap();
    app.refresh_permissions().unwrap();
    assert!(!app.snapshot().unwrap().receiving);
    std::fs::write(&status, "listen=false post=true wifi=false\n").unwrap();
    assert!(app.receive_at(false, port).is_err());
    assert!(!app.snapshot().unwrap().receiving);
    std::fs::write(&status, "listen=false post=true wifi=true\n").unwrap();
    app.receive_at(false, port).unwrap();
    assert!(app.snapshot().unwrap().receiving);
    std::fs::write(&status, "listen=false post=true wifi=false\n").unwrap();
    app.refresh_permissions().unwrap();
    assert!(!app.snapshot().unwrap().receiving);
    std::fs::write(&status, "listen=false post=true wifi=true\n").unwrap();
    app.receive_at(false, port).unwrap();
    std::fs::write(&status, "listen=true post=false wifi=true\n").unwrap();
    app.refresh_permissions().unwrap();
    assert!(!app.snapshot().unwrap().receiving);
    // A restored grant does not silently resume receiving.
    std::fs::write(&status, "listen=true post=true wifi=true\n").unwrap();
    app.refresh_permissions().unwrap();
    assert!(!app.snapshot().unwrap().receiving);
    app.stop_receiving().unwrap();
    wait(|| app.inner.lock().unwrap().listener.is_none());
}

#[test]
fn local_device_details_expose_only_public_metadata_without_enabling_receiving() {
    let temp = tempfile::tempdir().unwrap();
    let desktop = Desktop::new(temp.path().join("device"), temp.path().join("helper")).unwrap();
    let identity = Identity::generate();
    let fingerprint = identity.fingerprint();
    *desktop.identity.lock().unwrap() = Some(Arc::new(identity));
    let details = serde_json::to_value(desktop.local_device_info().unwrap()).unwrap();
    assert_eq!(details["identity"], fingerprint);
    assert_eq!(details["version"], env!("CARGO_PKG_VERSION"));
    assert!(!details["name"].as_str().unwrap().is_empty());
    assert_eq!(details.as_object().unwrap().len(), 3);
    let inner = desktop.inner.lock().unwrap();
    assert!(!inner.receiving_enabled);
    assert!(inner.listener.is_none());
    assert!(!inner.pairing_open);
}

#[test]
#[cfg(unix)]
fn receiving_preference_survives_restart_and_respects_permissions_and_explicit_off() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    let helper = temp.path().join("helper");
    let status = temp.path().join("status");
    std::fs::write(&helper, "#!/bin/sh\ncat \"$(dirname \"$0\")/status\"\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&status, "listen=false post=true wifi=true\n").unwrap();
    let new_app = || {
        let app = Desktop::new(root.clone(), helper.clone()).unwrap();
        *app.identity.lock().unwrap() = Some(Arc::new(Identity::generate()));
        app
    };
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let app = new_app();
    app.restore_receiving_at(port);
    assert!(!app.snapshot().unwrap().receiving);
    app.receive_at(true, port).unwrap();
    assert!(
        !preferences::load(&root).unwrap(),
        "pairing must not opt into receiving"
    );
    app.receive_at(false, port).unwrap();
    assert!(preferences::load(&root).unwrap());
    app.shutdown();
    wait(|| app.inner.lock().unwrap().listener.is_none());
    assert!(
        preferences::load(&root).unwrap(),
        "quitting must preserve the preference"
    );

    let restarted = new_app();
    restarted.restore_receiving_at(port);
    assert!(restarted.snapshot().unwrap().receiving);
    assert!(!restarted.snapshot().unwrap().discoverable);
    restarted.shutdown();
    wait(|| restarted.inner.lock().unwrap().listener.is_none());

    std::fs::write(&status, "listen=false post=true wifi=false\n").unwrap();
    let missing_wifi = new_app();
    missing_wifi.restore_receiving_at(port);
    assert!(!missing_wifi.snapshot().unwrap().receiving);
    assert!(missing_wifi.inner.lock().unwrap().listener.is_none());
    assert!(preferences::load(&root).unwrap());

    std::fs::write(&status, "listen=false post=false wifi=true\n").unwrap();
    let denied = new_app();
    denied.restore_receiving_at(port);
    assert!(!denied.snapshot().unwrap().receiving);
    assert!(denied.snapshot().unwrap().notification.is_none());
    assert!(denied.inner.lock().unwrap().listener.is_none());
    assert!(preferences::load(&root).unwrap());
    denied.stop_receiving().unwrap();
    std::fs::write(&status, "listen=false post=true wifi=true\n").unwrap();
    denied.restore_receiving_at(port);
    assert!(
        !denied.snapshot().unwrap().receiving,
        "late startup must not undo explicit off"
    );
    let off = new_app();
    off.restore_receiving_at(port);
    assert!(!off.snapshot().unwrap().receiving);

    let occupied = std::net::TcpListener::bind(("0.0.0.0", port)).unwrap();
    assert!(off.receive_at(false, port).is_err());
    assert!(
        !preferences::load(&root).unwrap(),
        "failed enable must not save on"
    );
    drop(occupied);
}

#[test]
fn wifi_permission_failure_opens_the_existing_gate_for_each_role() {
    for kind in [SessionKind::Incoming, SessionKind::Outgoing] {
        let temp = tempfile::tempdir().unwrap();
        let app = Desktop::new(temp.path().join("state"), temp.path().join("helper")).unwrap();
        let (id, _) = app.reserve(kind, None).unwrap();
        app.finish(
            id,
            Err(extend_computer_agent::low_jitter::PermissionRequired.into()),
        );
        let snapshot = app.snapshot().unwrap();
        assert!(snapshot.session.is_none());
        assert_eq!(
            snapshot.permission_request.as_deref(),
            Some(if kind == SessionKind::Incoming {
                "receive"
            } else {
                "share"
            })
        );
        assert!(snapshot.error.unwrap().contains("Wi-Fi optimization"));
        app.clear_message();
        assert!(app.snapshot().unwrap().permission_request.is_none());
    }
}

#[test]
fn custom_local_name_persists_and_resets_without_changing_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let helper = temp.path().join("helper");
    let app = Desktop::new(root.clone(), helper.clone()).unwrap();
    let identity = Identity::generate();
    let fingerprint = identity.fingerprint();
    *app.identity.lock().unwrap() = Some(Arc::new(identity));
    app.set_local_device_name("  Desk Mac  ".into()).unwrap();
    assert_eq!(app.local_device_info().unwrap().name, "Desk Mac");
    assert_eq!(app.local_device_info().unwrap().identity, fingerprint);
    assert!(app.set_local_device_name("Bad\nName".into()).is_err());
    let restarted = Desktop::new(root.clone(), helper.clone()).unwrap();
    assert_eq!(restarted.local_name(), "Desk Mac");
    restarted.set_local_device_name(" ".into()).unwrap();
    assert_eq!(
        Desktop::new(root, helper).unwrap().local_name(),
        peers::local_name()
    );
}

#[test]
fn snapshot_distinguishes_account_access_from_direct_pairing() {
    let temp = tempfile::tempdir().unwrap();
    let app = Desktop::new(temp.path().join("app"), temp.path().join("helper")).unwrap();
    app.set_test_identity(Identity::generate());
    let paired = "a".repeat(64);
    let account = "b".repeat(64);
    let both = "c".repeat(64);
    app.store().unwrap().remember(&paired).unwrap();
    app.store().unwrap().remember(&both).unwrap();
    app.sync_account_peers(
        "server/account",
        &[
            serde_json::json!({"id":"account", "fingerprint":account, "key_verified":true}),
            serde_json::json!({"id":"both", "fingerprint":both, "key_verified":true}),
        ],
    )
    .unwrap();

    let snapshot = app.snapshot().unwrap();
    let sources: BTreeMap<_, _> = snapshot
        .peers
        .iter()
        .map(|peer| (peer.id.clone(), peer.trust_source))
        .collect();
    assert_eq!(sources.len(), 3);
    assert_eq!(sources[&paired], TrustSource::Pairing);
    assert_eq!(sources[&account], TrustSource::Account);
    assert_eq!(sources[&both], TrustSource::Pairing);
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(serialized["peers"][0]["trust_source"], "pairing");
    assert_eq!(serialized["peers"][1]["trust_source"], "account");

    app.clear_account_peers();
    let snapshot = app.snapshot().unwrap();
    assert_eq!(snapshot.peers.len(), 2);
    assert!(snapshot
        .peers
        .iter()
        .all(|peer| peer.trust_source == TrustSource::Pairing));
    assert!(!snapshot.peers.iter().any(|peer| peer.id == account));
}

#[test]
#[cfg(unix)]
fn account_control_connects_without_approval_and_ends_on_signout() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let helper = temp.path().join("helper");
    std::fs::write(
        &helper,
        r#"#!/bin/sh
case "$1" in
status) if [ -e "${0%/*}/deny-post" ]; then echo 'listen=true post=false wifi=true'; else echo 'listen=true post=true wifi=true'; fi;;
inject-control) echo 'READY 1728 1117'; while IFS= read -r line; do echo OK; done;;
capture-control-*) echo 'READY 1512 982'; while IFS= read -r line; do :; done;;
*) exit 1;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let sender_helper_dir = temp.path().join("sender-helper");
    std::fs::create_dir(&sender_helper_dir).unwrap();
    let sender_helper = sender_helper_dir.join("helper");
    std::fs::copy(&helper, &sender_helper).unwrap();
    let receiver = Desktop::new(temp.path().join("receiver"), helper.clone()).unwrap();
    let sender = Desktop::new(temp.path().join("sender"), sender_helper).unwrap();
    receiver.set_test_identity(Identity::generate());
    sender.set_test_identity(Identity::generate());
    let receiver_id = receiver.identity().unwrap().fingerprint();
    let sender_id = sender.identity().unwrap().fingerprint();
    receiver.sync_account_peers("server/account",&[serde_json::json!({"id":"a".repeat(64),"fingerprint":sender_id,"name":"Sender","key_verified":true})]).unwrap();
    sender.sync_account_peers("server/account",&[serde_json::json!({"id":"b".repeat(64),"fingerprint":receiver_id,"name":"Receiver","key_verified":true})]).unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    receiver.receive_at(false, port).unwrap();
    let device = Device {
        name: "Receiver".into(),
        address: format!("127.0.0.1:{port}"),
        edge: "left".into(),
    };
    sender.save_device(&receiver_id, device.clone()).unwrap();
    sender.connect(receiver_id.clone(), device.clone()).unwrap();
    wait(|| {
        sender
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|session| session.phase == Phase::Connected)
    });
    assert!(receiver.approvals.current().is_none());
    assert!(sender.approvals.current().is_none());
    assert!(
        TrustStore::open(&receiver.root)
            .unwrap()
            .peer(&sender_id)
            .unwrap()
            .is_none(),
        "Account membership must never become permanent local pairing"
    );
    sender.disconnect();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    receiver.inner.lock().unwrap().receiving_enabled = false;
    sender.connect(receiver_id.clone(), device.clone()).unwrap();
    wait(|| sender.snapshot().unwrap().session.is_none());
    assert!(receiver.approvals.current().is_none());
    wait(|| receiver.snapshot().unwrap().session.is_none());
    receiver.inner.lock().unwrap().receiving_enabled = true;
    std::fs::write(temp.path().join("deny-post"), "").unwrap();
    sender.connect(receiver_id.clone(), device.clone()).unwrap();
    wait(|| {
        sender.snapshot().unwrap().session.is_none()
            && receiver.snapshot().unwrap().session.is_none()
    });
    assert!(!receiver.snapshot().unwrap().receiving);
    assert!(receiver.approvals.current().is_none());
    std::fs::remove_file(temp.path().join("deny-post")).unwrap();
    receiver.inner.lock().unwrap().receiving_enabled = true;
    sender.connect(receiver_id, device).unwrap();
    wait(|| {
        sender
            .snapshot()
            .unwrap()
            .session
            .is_some_and(|session| session.phase == Phase::Connected)
    });
    assert!(receiver.approvals.current().is_none());
    receiver.clear_account_peers();
    wait(|| receiver.snapshot().unwrap().session.is_none());
    sender.shutdown();
    receiver.shutdown();
}

#[test]
fn installation_reservation_blocks_new_sessions_and_can_be_canceled() {
    let temp = tempfile::tempdir().unwrap();
    let app = Desktop::new(temp.path().join("app"), temp.path().join("helper")).unwrap();
    let (id, _) = app.reserve(SessionKind::Pair, None).unwrap();
    assert!(!app.prepare_update());
    app.finish(id, Ok(()));
    assert!(app.prepare_update());
    assert!(app.reserve(SessionKind::Pair, None).is_err());
    app.cancel_update();
    assert!(app.reserve(SessionKind::Pair, None).is_ok());
    app.shutdown();
}
