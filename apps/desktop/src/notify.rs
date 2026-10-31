//! System notifications for the UI: new mail, and calendar alerts on hosts
//! that don't schedule them with the OS. In a macOS app bundle everything
//! goes through the User Notifications framework (`mac_alerts`); elsewhere
//! (other platforms, a bare dev binary) through the notification plugin.

use tauri_plugin_notification::{NotificationExt, PermissionState};

/// Whether calendar alerts are scheduled with the OS here, so the UI
/// doesn't show its own notifications for them (0057).
#[tauri::command]
pub fn os_alerts() -> bool {
    #[cfg(target_os = "macos")]
    return crate::mac_alerts::available();
    #[cfg(not(target_os = "macos"))]
    false
}

/// Whether notifications may show; `ask` asks the user if they haven't said.
#[tauri::command]
pub async fn can_notify(app: tauri::AppHandle, ask: bool) -> bool {
    #[cfg(target_os = "macos")]
    if crate::mac_alerts::available() {
        return if ask { crate::mac_alerts::ask_permission().await } else { crate::mac_alerts::allowed().await };
    }
    let n = app.notification();
    match n.permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Denied) => false,
        _ if ask => matches!(n.request_permission(), Ok(PermissionState::Granted)),
        _ => false,
    }
}

/// Shows a notification now.
#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) {
    #[cfg(target_os = "macos")]
    if crate::mac_alerts::available() {
        return crate::mac_alerts::notify(&title, &body);
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("notification: {e}");
    }
}

/// The event a clicked alert asked to open before the UI was listening.
#[tauri::command]
pub fn take_open_request() -> Option<serde_json::Value> {
    #[cfg(target_os = "macos")]
    return crate::mac_alerts::take_open_request();
    #[cfg(not(target_os = "macos"))]
    None
}
