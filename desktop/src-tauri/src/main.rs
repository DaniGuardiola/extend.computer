#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod accounts;
mod approval;
mod discovery;
mod native;
#[cfg(target_os = "macos")]
mod passkey;
mod peers;
mod permission_flow;
mod relay;
mod runtime;
#[cfg(target_os = "macos")]
mod traffic_lights;
mod updates;
mod wifi_permission;
use peers::Device;
use runtime::{Desktop, LocalDeviceInfo, Snapshot};
use std::sync::Arc;
use tauri::Manager;
type State<'a> = tauri::State<'a, Arc<Desktop>>;
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
async fn background<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(error)?
        .map_err(|e| runtime::friendly_error(&e))
}
#[tauri::command]
fn snapshot(state: State<'_>) -> Result<Snapshot, String> {
    state.snapshot().map_err(error)
}
#[tauri::command]
async fn set_local_device_name(state: State<'_>, name: String) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.set_local_device_name(name)).await
}
#[tauri::command]
async fn local_device_info(state: State<'_>) -> Result<LocalDeviceInfo, String> {
    let state = state.inner().clone();
    background(move || state.local_device_info()).await
}
#[tauri::command]
fn pair_device(state: State<'_>, device: Device, code: String) -> Result<(), String> {
    state.inner().pair(device, code).map_err(error)
}
#[tauri::command]
fn pair_nearby(state: State<'_>, device: Device) -> Result<(), String> {
    state.inner().pair_nearby(device).map_err(error)
}
#[tauri::command]
fn close_pairing(state: State<'_>) {
    state.close_pairing();
}
#[tauri::command]
async fn connect_device(state: State<'_>, peer: String, device: Device) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.connect(peer, device)).await
}
#[tauri::command]
async fn connect_display(
    state: State<'_>,
    peer: String,
    device: Device,
    config: extend_computer_agent::session::display::Config,
) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.connect_display(peer, device, config)).await
}
#[tauri::command]
async fn update_display(
    state: State<'_>,
    session: u64,
    config: extend_computer_agent::session::display::Config,
) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.update_display(session, config)).await
}
#[tauri::command]
async fn connect_usb(
    state: State<'_>,
    peer: String,
    vendor: String,
    serial: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.connect_usb(&peer, &vendor, &serial)).await
}
#[tauri::command]
async fn disconnect_usb(state: State<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || {
        state.disconnect_usb();
        Ok(())
    })
    .await
}
#[tauri::command]
async fn usb_devices(state: State<'_>) -> Result<serde_json::Value, String> {
    let state = state.inner().clone();
    background(move || state.usb_devices()).await
}
#[tauri::command]
fn usb_status(state: State<'_>) -> serde_json::Value {
    state.usb_status()
}
#[tauri::command]
fn disconnect(state: State<'_>) {
    state.end_session();
}
#[tauri::command]
async fn start_receiving(state: State<'_>, pairing: bool) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.receive(pairing)).await
}
#[tauri::command]
async fn stop_receiving(state: State<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.stop_receiving()).await
}
#[tauri::command]
fn answer_request(state: State<'_>, id: u64, answer: String) -> Result<(), String> {
    let answer = match answer.as_str() {
        "remember" => approval::Answer::Remember,
        "deny" => approval::Answer::Deny,
        _ => return Err("Unknown response".into()),
    };
    state.approvals.answer(id, answer).map_err(error)
}
#[tauri::command]
fn update_device(state: State<'_>, peer: String, device: Device) -> Result<(), String> {
    state.save_device(&peer, device).map_err(error)
}
#[tauri::command]
async fn unpair_device(state: State<'_>, peer: String) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.unpair(&peer)).await
}
#[tauri::command]
fn dismiss_notification(state: State<'_>, id: u64) {
    state.dismiss_notification(id);
}
#[tauri::command]
async fn dismiss_removed(state: State<'_>, peer: String) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.dismiss_removed(&peer)).await
}
#[tauri::command]
async fn check_permissions(state: State<'_>) -> Result<native::Permissions, String> {
    let state = state.inner().clone();
    background(move || state.refresh_permissions()).await
}
#[tauri::command]
async fn open_permission(
    app: tauri::AppHandle,
    state: State<'_>,
    permission: String,
) -> Result<(), String> {
    let repair_wifi = if permission == "wifi" {
        background(|| Ok(!extend_computer_agent::low_jitter::ready())).await?
    } else {
        false
    };
    if repair_wifi && state.snapshot().map_err(error)?.session.is_some() {
        return Err("Disconnect before changing Wi-Fi optimization permissions.".into());
    }
    let resources = permission_flow::resources(&app).map_err(error)?;
    let (send, receive) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = send.send(if permission == "wifi" {
            wifi_permission::open(repair_wifi)
        } else {
            permission_flow::open(&resources, &permission)
        });
    })
    .map_err(error)?;
    background(move || receive.recv().map_err(anyhow::Error::from)?).await
}
#[tauri::command]
fn dismiss_message(state: State<'_>) {
    state.clear_message();
}

type AccountState<'a> = tauri::State<'a, Arc<accounts::Accounts>>;
#[tauri::command]
async fn account_status(state: AccountState<'_>) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || Ok(state.view())).await
}
#[tauri::command]
async fn account_refresh(state: AccountState<'_>) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || state.refresh()).await
}
#[tauri::command]
async fn account_configure(
    state: AccountState<'_>,
    server: String,
) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || state.configure(server)).await
}
#[tauri::command]
async fn account_login(
    state: AccountState<'_>,
    email: String,
    password: String,
) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || state.login(email, password)).await
}
#[tauri::command]
fn account_passkey_available() -> bool {
    #[cfg(target_os = "macos")]
    return passkey::available();
    #[cfg(not(target_os = "macos"))]
    false
}
#[tauri::command]
async fn account_passkey(
    app: tauri::AppHandle,
    state: AccountState<'_>,
) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || {
        #[cfg(target_os = "macos")]
        {
            let (challenge, handle) = state.passkey_options()?;
            let credential = passkey::sign_in(&app, &challenge)?;
            state.passkey_login(handle, credential)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app, state);
            anyhow::bail!("Passkeys in the app are not available on this platform.")
        }
    })
    .await
}
#[tauri::command]
async fn account_verify(state: AccountState<'_>, code: String) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || state.verify(code)).await
}
#[tauri::command]
async fn account_logout(state: AccountState<'_>) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || state.logout()).await
}
#[tauri::command]
async fn account_remove_device(
    state: AccountState<'_>,
    id: String,
) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || state.remove(id)).await
}
#[tauri::command]
async fn account_cancel(state: AccountState<'_>) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || Ok(state.cancel())).await
}
#[tauri::command]
async fn account_browser_login(
    app: tauri::AppHandle,
    state: AccountState<'_>,
) -> Result<accounts::View, String> {
    let state = state.inner().clone();
    background(move || {
        state.browser(move || {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            });
        })
    })
    .await
}
#[tauri::command]
async fn account_open_page(state: AccountState<'_>, page: String) -> Result<(), String> {
    let state = state.inner().clone();
    background(move || state.open_page(page)).await
}

#[cfg(any(debug_assertions, feature = "display-benchmark", test))]
fn display_benchmark_arguments(
    args: &[String],
) -> anyhow::Result<(String, extend_computer_agent::session::display::Config, u64)> {
    use extend_computer_agent::session::display::{Config, Mode, VideoCodec};
    if !matches!(args.len(), 6 | 7 | 9 | 10)
        || !args.get(1).is_some_and(|arg| arg == "--display-benchmark")
        || args
            .get(6)
            .is_some_and(|arg| !matches!(arg.as_str(), "auto" | "retina" | "native"))
        || args
            .get(9)
            .is_some_and(|arg| !matches!(arg.as_str(), "h264" | "hevc"))
    {
        anyhow::bail!("Use --display-benchmark PEER WIDTH HEIGHT SECONDS [auto|retina|native [FPS BITRATE [h264|hevc]]]");
    }
    let auto_size = args.get(6).is_some_and(|arg| arg == "auto");
    let codec = if args.get(9).is_some_and(|arg| arg == "h264") {
        VideoCodec::H264
    } else {
        VideoCodec::Hevc
    };
    Ok((
        args[2].clone(),
        Config {
            mode: Mode::Extend,
            width: args[3].parse()?,
            height: args[4].parse()?,
            fps: args
                .get(7)
                .map(|arg| arg.parse())
                .transpose()?
                .unwrap_or(60),
            bitrate: args
                .get(8)
                .map(|arg| arg.parse())
                .transpose()?
                .unwrap_or(20_000_000),
            retina: auto_size || args.get(6).is_some_and(|arg| arg == "retina"),
            auto_size,
            codec,
            separate_cursor: true,
        },
        args[5].parse()?,
    ))
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            if std::env::var_os("EXTEND_DISPLAY_METRICS").is_some()
                || std::env::var_os("EXTEND_DISPLAY_BENCHMARK_CONTEXT").is_some()
            {
                eprintln!(
                    "Display build {}",
                    serde_json::json!({
                        "debug_assertions": cfg!(debug_assertions),
                        "development_identity": cfg!(feature = "dev-identity"),
                        "benchmark_feature": cfg!(feature = "display-benchmark"),
                        "configured_windows": app.config().app.windows.len(),
                        "created_webviews": app.webview_windows().len(),
                        "webview_labels": app.webview_windows().keys().cloned().collect::<Vec<_>>()
                    })
                );
            }
            #[cfg(feature = "dev-identity")]
            let profile = "extend.computer Development";
            #[cfg(not(feature = "dev-identity"))]
            let profile = "extend.computer";
            let root = app.path().data_dir()?.join(profile);
            let resources = permission_flow::resources(app.handle())?;
            wifi_permission::initialize(&resources)?;
            let executable = std::env::current_exe()?;
            let contents = executable
                .parent()
                .and_then(|p| p.parent())
                .ok_or("Missing app bundle")?;
            extend_computer_agent::low_jitter::configure_app_helper(
                contents.join("Library/LaunchServices/ExtendComputerLowJitter"),
            )?;
            let packaged = app
                .path()
                .resource_dir()?
                .join("helpers/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor");
            #[cfg(debug_assertions)]
            let packaged = if packaged.is_file() {
                packaged
            } else {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                    "../../target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor",
                )
            };
            let desktop = Desktop::new(root, packaged)?;
            desktop.start_presence()?;
            desktop.start_display_listener()?;
            desktop.start_peer_checks();
            desktop.restore_receiving();
            let accounts = accounts::Accounts::new(desktop.clone())?;
            accounts.start();
            #[cfg(any(debug_assertions, feature = "display-benchmark"))]
            {
                let args: Vec<_> = std::env::args().collect();
                if args.get(1).is_some_and(|arg| arg == "--usb-media-test") {
                    if args.len() != 7 {
                        return Err(
                            "Use --usb-media-test PEER VENDOR SERIAL RECEIPT SECONDS".into()
                        );
                    }
                    desktop.start_usb_media_test(
                        args[2].clone(),
                        args[3].clone(),
                        args[4].clone(),
                        args[5].clone().into(),
                        args[6].parse()?,
                    );
                }
                if args.get(1).is_some_and(|arg| arg == "--usb-control-test") {
                    if args.len() != 6 {
                        return Err("Use --usb-control-test PEER VENDOR SERIAL RECEIPT".into());
                    }
                    desktop.start_usb_control_test(
                        args[2].clone(),
                        args[3].clone(),
                        args[4].clone(),
                        args[5].clone().into(),
                    )?;
                }
                if args.get(1).is_some_and(|arg| arg == "--display-benchmark") {
                    let (peer, config, seconds) = display_benchmark_arguments(&args)?;
                    desktop.start_display_benchmark(peer, config, seconds);
                }
            }
            app.manage(accounts);
            app.manage(desktop.clone());
            updates::initialize(app.handle(), &desktop)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect_display,
            update_display,
            connect_usb,
            disconnect_usb,
            usb_status,
            usb_devices,
            updates::update_settings,
            updates::check_for_updates,
            updates::configure_updates,
            account_status,
            account_refresh,
            account_configure,
            account_login,
            account_passkey,
            account_passkey_available,
            account_verify,
            account_logout,
            account_remove_device,
            account_cancel,
            account_browser_login,
            account_open_page,
            snapshot,
            local_device_info,
            set_local_device_name,
            discovery::discover,
            pair_device,
            pair_nearby,
            close_pairing,
            connect_device,
            disconnect,
            start_receiving,
            stop_receiving,
            answer_request,
            update_device,
            unpair_device,
            dismiss_removed,
            dismiss_notification,
            check_permissions,
            open_permission,
            dismiss_message
        ])
        .build(tauri::generate_context!())
        .expect("Could not run extend.computer")
        .run(|app, event| {
            if matches!(&event, tauri::RunEvent::Ready) {
                updates::install_menu();
            }
            #[cfg(target_os = "macos")]
            if matches!(
                &event,
                tauri::RunEvent::Ready
                    | tauri::RunEvent::WindowEvent {
                        event: tauri::WindowEvent::Focused(true) | tauri::WindowEvent::Resized(_),
                        ..
                    }
            ) {
                traffic_lights::apply(app);
            }
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Some(state) = app.try_state::<Arc<Desktop>>() {
                    state.shutdown();
                }
            }
        });
}

#[cfg(test)]
mod benchmark_argument_tests {
    use super::display_benchmark_arguments;
    use extend_computer_agent::session::display::VideoCodec;

    fn arguments(extra: &[&str]) -> Vec<String> {
        [
            "app",
            "--display-benchmark",
            "account",
            "1728",
            "1085",
            "120",
        ]
        .into_iter()
        .chain(extra.iter().copied())
        .map(String::from)
        .collect()
    }

    #[test]
    fn preserves_hevc_default_and_explicit_benchmark_codec() {
        let (_, defaults, seconds) = display_benchmark_arguments(&arguments(&[])).unwrap();
        assert_eq!(defaults.codec, VideoCodec::Hevc);
        assert_eq!(
            (defaults.fps, defaults.bitrate, seconds),
            (60, 20_000_000, 120)
        );
        assert!(!defaults.retina && !defaults.auto_size);
        for (name, codec) in [("h264", VideoCodec::H264), ("hevc", VideoCodec::Hevc)] {
            let (_, config, _) =
                display_benchmark_arguments(&arguments(&["retina", "60", "40000000", name]))
                    .unwrap();
            assert_eq!(config.codec, codec);
            assert_eq!(config.bitrate, 40_000_000);
            assert!(config.retina && !config.auto_size && config.separate_cursor);
        }
        let (_, automatic, _) = display_benchmark_arguments(&arguments(&["auto"])).unwrap();
        assert!(automatic.retina && automatic.auto_size);
    }

    #[test]
    fn rejects_partial_options_invalid_codec_and_extra_arguments() {
        for extra in [
            vec!["retina", "60"],
            vec!["retina", "60", "40000000", "h265"],
            vec!["retina", "60", "40000000", "H264"],
            vec!["retina", "60", "40000000", "h264", "extra"],
            vec!["unsupported"],
            vec!["retina", "fps", "40000000", "h264"],
        ] {
            assert!(
                display_benchmark_arguments(&arguments(&extra)).is_err(),
                "{extra:?}"
            );
        }
        let mut incomplete = arguments(&[]);
        incomplete.pop();
        assert!(display_benchmark_arguments(&incomplete).is_err());
    }
}
