mod gateway;

use gateway::{GatewayProfile, GatewayState, NetworkLogEntry};
use tauri::Manager;

#[tauri::command]
fn gateway_list_profiles(state: tauri::State<'_, GatewayState>) -> Vec<GatewayProfile> {
    state.list_profiles()
}

#[tauri::command]
fn gateway_upsert_profile(
    state: tauri::State<'_, GatewayState>,
    profile: GatewayProfile,
) -> Result<GatewayProfile, String> {
    state.upsert_profile(profile)
}

#[tauri::command]
fn gateway_delete_profile(
    state: tauri::State<'_, GatewayState>,
    id: String,
) -> Result<(), String> {
    state.delete_profile(id)
}

#[tauri::command]
fn gateway_set_active_profile(
    state: tauri::State<'_, GatewayState>,
    id: String,
) -> Result<(), String> {
    state.set_active_profile(id)
}

#[tauri::command]
fn gateway_get_status(
    state: tauri::State<'_, GatewayState>,
) -> gateway::manager::GatewayStatus {
    state.status()
}

#[tauri::command]
fn gateway_get_logs(
    state: tauri::State<'_, GatewayState>,
    limit: usize,
    profile_id: Option<String>,
) -> Vec<String> {
    state.get_logs(limit, profile_id)
}

#[tauri::command]
fn gateway_get_network_logs(
    state: tauri::State<'_, GatewayState>,
    profile_id: Option<String>,
    limit: usize,
) -> Vec<NetworkLogEntry> {
    state.get_network_logs(profile_id, limit)
}

#[tauri::command]
fn gateway_clear_network_logs(state: tauri::State<'_, GatewayState>, profile_id: Option<String>) {
    state.clear_network_logs(profile_id)
}

#[tauri::command]
async fn gateway_connect(
    state: tauri::State<'_, GatewayState>,
    profile_id: Option<String>,
    upstream_proxy: Option<String>,
) -> Result<gateway::manager::GatewayStatus, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || state.connect(profile_id, upstream_proxy))
        .await
        .map_err(|e| format!("连接任务异常: {e}"))?
}

#[tauri::command]
async fn gateway_disconnect(
    state: tauri::State<'_, GatewayState>,
    profile_id: Option<String>,
) -> Result<gateway::manager::GatewayStatus, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || state.disconnect(profile_id))
        .await
        .map_err(|e| format!("断开任务异常: {e}"))?
}

#[tauri::command]
async fn gateway_poll(
    state: tauri::State<'_, GatewayState>,
) -> Result<gateway::manager::GatewayStatus, String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || state.poll_and_maybe_reconnect())
        .await
        .map_err(|e| format!("轮询任务异常: {e}"))?
}

#[tauri::command]
fn gateway_set_reconnect(
    state: tauri::State<'_, GatewayState>,
    profile_id: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    state.set_reconnect(profile_id, enabled)
}

#[tauri::command]
fn gateway_new_profile() -> GatewayProfile {
    GatewayProfile::new_blank("新服务器")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = GatewayState::new(app.handle());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            gateway_list_profiles,
            gateway_upsert_profile,
            gateway_delete_profile,
            gateway_set_active_profile,
            gateway_get_status,
            gateway_get_logs,
            gateway_get_network_logs,
            gateway_clear_network_logs,
            gateway_connect,
            gateway_disconnect,
            gateway_poll,
            gateway_set_reconnect,
            gateway_new_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
