#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod accounts;
mod approval;
mod discovery;
mod native;
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
fn disconnect(state: State<'_>) {
    state.disconnect();
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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
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
            desktop.start_peer_checks();
            desktop.restore_receiving();
            let accounts = accounts::Accounts::new(desktop.clone())?;
            accounts.start();
            app.manage(accounts);
            app.manage(desktop.clone());
            updates::initialize(app.handle(), &desktop)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            updates::update_settings,
            updates::check_for_updates,
            updates::configure_updates,
            account_status,
            account_refresh,
            account_configure,
            account_login,
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
