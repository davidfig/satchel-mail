//! The count on Linux docks. Docks that show counts (Ubuntu's, KDE's and others)
//! listen for this D-Bus signal. Tauri's `set_badge_count` needs libunity and the
//! old Unity desktop, so most Linux desktops would never see it (0071).

use std::collections::HashMap;

use zbus::Connection;
use zbus::zvariant::Value;

/// Names the desktop file the count is for. The bundler names it after
/// `productName` in `tauri.conf.json`.
const APP_URI: &str = "application://satchel.email.desktop";
const PATH: &str = "/com/canonical/unity/launcherentry/satchel_email";
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

/// Keeps its bus connection open: docks drop the count when the sender goes away.
pub struct Launcher {
    connection: Option<Connection>,
}

impl Launcher {
    pub fn new<W>(_window: W) -> Self {
        Self { connection: None }
    }

    pub async fn show(&mut self, count: i64) -> Result<(), String> {
        let connection = match &self.connection {
            Some(c) => c.clone(),
            None => Connection::session().await.map_err(|e| e.to_string())?,
        };
        let properties = HashMap::from([("count", Value::from(count)), ("count-visible", Value::from(count > 0))]);
        // A failed send forgets the connection, so the next count opens a new one.
        self.connection = None;
        connection.emit_signal(None::<()>, PATH, INTERFACE, "Update", &(APP_URI, properties)).await.map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        Ok(())
    }
}
