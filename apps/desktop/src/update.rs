//! Updates from the release feed (0063). Release builds with the `updater`
//! feature check the feed at start and every six hours, download a newer
//! version in the background and check its signature, then tell the UI
//! (`satchel://update-ready`), which offers to restart into it. Nothing
//! installs until the user says so. Other builds have no feed: the commands
//! answer that there's nothing to install.

/// The version downloaded and waiting to be installed, if any. For a UI that
/// loaded after `satchel://update-ready` was sent.
#[tauri::command]
pub fn update_ready(app: tauri::AppHandle) -> Option<String> {
    #[cfg(feature = "updater")]
    return feed::ready(&app);
    #[cfg(not(feature = "updater"))]
    {
        let _ = app;
        None
    }
}

/// What a check the user asked for found.
#[derive(serde::Serialize)]
pub struct UpdateCheck {
    /// The version running now.
    current: String,
    /// The newer version downloaded and waiting to be installed, if any.
    ready: Option<String>,
}

/// Checks the feed now, and downloads a newer version if there is one. For
/// Help > Check for Updates; the same check runs by itself every six hours.
#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateCheck, String> {
    #[cfg(feature = "updater")]
    {
        feed::check(&app).await.map_err(|e| format!("{e:#}"))?;
        Ok(UpdateCheck { current: app.package_info().version.to_string(), ready: feed::ready(&app) })
    }
    #[cfg(not(feature = "updater"))]
    {
        let _ = app;
        Err("this build doesn't update itself".into())
    }
}

/// Installs the downloaded update and restarts into it.
#[tauri::command]
pub fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(feature = "updater")]
    return feed::install(&app);
    #[cfg(not(feature = "updater"))]
    {
        let _ = app;
        Err("this build doesn't update itself".into())
    }
}

/// Starts checking the feed, if this build has one.
pub fn start(app: &tauri::AppHandle) {
    #[cfg(feature = "updater")]
    feed::start(app);
    #[cfg(not(feature = "updater"))]
    let _ = app;
}

#[cfg(feature = "updater")]
mod feed {
    use std::sync::Mutex;
    use std::time::Duration;

    use tauri::{Emitter, Manager};
    use tauri_plugin_updater::{Update, UpdaterExt};

    const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

    /// A downloaded update whose signature checked out, with its bytes.
    #[derive(Default)]
    struct Staged(Mutex<Option<(Update, Vec<u8>)>>);

    /// Held for a whole check, so the user's check and the timer's never
    /// download the same update twice: the second waits, then finds it staged.
    #[derive(Default)]
    struct Checking(tauri::async_runtime::Mutex<()>);

    /// How far a download has got, for the UI to show while the user waits
    /// on a check they asked for (`satchel://update-progress`).
    #[derive(Clone, serde::Serialize)]
    struct Progress<'a> {
        version: &'a str,
        /// None when the server doesn't say how big the download is.
        percent: Option<u64>,
    }

    pub fn start(app: &tauri::AppHandle) {
        app.manage(Checking::default());
        app.manage(Staged::default());
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                if let Err(e) = check(&app).await {
                    tracing::warn!("update check: {e:#}");
                }
                tokio::time::sleep(EVERY).await;
            }
        });
    }

    pub async fn check(app: &tauri::AppHandle) -> anyhow::Result<()> {
        let checking = app.state::<Checking>();
        let _one_at_a_time = checking.0.lock().await;
        let Some(update) = app.updater()?.check().await? else { return Ok(()) };
        if ready(app).as_deref() == Some(update.version.as_str()) {
            return Ok(());
        }
        let version = update.version.clone();
        // Sent when the whole percent changes, not for every chunk.
        let (mut got, mut shown) = (0u64, None);
        let mut progress = |chunk: usize, total: Option<u64>| {
            got += chunk as u64;
            let percent = total.filter(|t| *t > 0).map(|t| (got * 100 / t).min(100));
            if shown != Some(percent) {
                shown = Some(percent);
                let _ = app.emit_to("main", "satchel://update-progress", Progress { version: &version, percent });
            }
        };
        progress(0, None);
        // `download` checks the signature against the public key built into the app.
        let bytes = update.download(progress, || {}).await?;
        tracing::info!("update {version} downloaded");
        *app.state::<Staged>().0.lock().unwrap() = Some((update, bytes));
        app.emit_to("main", "satchel://update-ready", &version)?;
        Ok(())
    }

    pub fn ready(app: &tauri::AppHandle) -> Option<String> {
        app.state::<Staged>().0.lock().unwrap().as_ref().map(|(u, _)| u.version.clone())
    }

    pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
        let Some((update, bytes)) = app.state::<Staged>().0.lock().unwrap().take() else {
            return Err("no update has been downloaded".into());
        };
        // On Windows the installer takes over from here and quits the app itself.
        update.install(bytes).map_err(|e| e.to_string())?;
        app.restart()
    }
}
