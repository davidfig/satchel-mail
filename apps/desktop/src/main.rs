//! Satchel Mail desktop host: owns the engine, bridges the UI over Tauri IPC
//! and forwards core events. All product logic lives in satchel-core.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
mod menu;

use std::sync::Arc;

use base64::Engine as _;
use satchel_api::{Request, Response};
use satchel_core::secrets::{FileSecretStore, KeychainSecretStore, SecretStore};
use satchel_core::{Caller, Engine, EngineOptions, Mode};
use tauri::webview::NewWindowResponse;
use tauri::{Emitter, Manager, State, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    engine: Arc<Engine>,
    _lock: std::fs::File,
}

#[tauri::command]
async fn api(state: State<'_, AppState>, request: Request) -> Result<Response, String> {
    let id = request.id;
    Ok(match state.engine.handle(&Caller::User, request.command).await {
        Ok(v) => Response { id, ok: Some(v), error: None },
        Err(e) => Response { id, ok: None, error: Some(e) },
    })
}

/// Open http(s)/mailto links in the system browser. Nothing else is allowed.
#[tauri::command]
fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")) {
        return Err("unsupported link".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Ask the user to pick a folder, starting in `start` if it exists. Returns
/// the chosen path, or nothing if they cancelled.
#[tauri::command]
async fn choose_folder(app: tauri::AppHandle, title: String, start: Option<String>) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_title(title).set_can_create_directories(true);
    if let Some(dir) = start.map(std::path::PathBuf::from).filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else { return Ok(None) };
    Ok(Some(path.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned()))
}

/// Ask where to save an attachment, then write it. Returns the chosen path.
#[tauri::command]
async fn save_file(app: tauri::AppHandle, filename: String, data_base64: String) -> Result<Option<String>, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_base64).map_err(|e| e.to_string())?;
    // Without the invisible marks that make "invoice\u{202E}fdp.exe" read as "invoiceexe.pdf".
    let safe: String = satchel_mime::clean_filename(&filename).chars().map(|c| if "/\\:".contains(c) { '_' } else { c }).collect();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(&safe).save_file(move |p| {
        let _ = tx.send(p);
    });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    // Mark it as downloaded, so the OS checks it (Gatekeeper, SmartScreen)
    // when it's opened. A file we can't mark isn't left behind unchecked,
    // unless the disk can't hold the mark at all (FAT, some network shares).
    if let Err(e) = mark_downloaded(&path)
        && !matches!(e.kind(), std::io::ErrorKind::Unsupported | std::io::ErrorKind::InvalidFilename)
    {
        let _ = std::fs::remove_file(&path);
        return Err(format!("couldn't mark the saved file as downloaded from the internet: {e}"));
    }
    Ok(Some(path.display().to_string()))
}

/// macOS: the `com.apple.quarantine` attribute browsers set
/// ("flags;hex time;agent;event id"; 0081 = downloaded, not yet approved).
#[cfg(target_os = "macos")]
fn mark_downloaded(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let value = format!("0081;{secs:08x};Satchel Mail;");
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: both strings are NUL-terminated and `value` outlives the call.
    let r = unsafe { libc::setxattr(path.as_ptr(), c"com.apple.quarantine".as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Windows: the Mark of the Web, a `Zone.Identifier` stream saying the file
/// came from the Internet zone (3).
#[cfg(windows)]
fn mark_downloaded(path: &std::path::Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
}

#[cfg(not(any(target_os = "macos", windows)))]
fn mark_downloaded(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// Absolute path of the bundled `satchel` CLI (used in MCP setup snippets).
#[tauri::command]
fn cli_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) { "satchel.exe" } else { "satchel" };
    let p = dir.join(name);
    p.exists().then(|| p.display().to_string())
}

fn lock(dir: &std::path::Path) -> anyhow::Result<Option<std::fs::File>> {
    std::fs::create_dir_all(dir)?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("engine.lock"))?;
    match f.try_lock() {
        Ok(()) => Ok(Some(f)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// The window-state plugin only writes to disk on a clean quit, so a crash,
/// force-quit or `tauri dev` restart forgets where the window was. Also save
/// half a second after the window stops moving or resizing.
fn save_window_state_on_change(window: &tauri::WebviewWindow) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri_plugin_window_state::{AppHandleExt, StateFlags};
    let changes = Arc::new(AtomicU64::new(0));
    let app = window.app_handle().clone();
    window.on_window_event(move |e| {
        if !matches!(e, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
            return;
        }
        let n = changes.fetch_add(1, Ordering::SeqCst) + 1;
        let (changes, app) = (changes.clone(), app.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if changes.load(Ordering::SeqCst) == n {
                let _ = app.save_window_state(StateFlags::all());
            }
        });
    });
}

/// Providers for the engine; on macOS, places come from Apple Maps (0050).
fn factory(app: &tauri::AppHandle) -> satchel_providers::Factory {
    let f = satchel_providers::Factory::new();
    #[cfg(target_os = "macos")]
    let f = {
        let app = app.clone();
        let maps = satchel_providers::places::apple_maps::AppleMaps::new(Box::new(move |job| Ok(app.run_on_main_thread(job)?)));
        f.with_places(Arc::new(maps))
    };
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    f
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tantivy=warn".into()))
        .init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Reopen the window where and at the size the user last left it.
        .plugin(tauri_plugin_window_state::Builder::default().build());
    #[cfg(feature = "updater")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(|app, e| menu::forward(app, e.id().as_ref()));
    builder
        .setup(|app| {
            let data_dir = satchel_core::default_data_dir();
            let Some(lock) = lock(&data_dir)? else {
                app.dialog()
                    .message(format!("Satchel Mail is already running for {} (perhaps `satchel serve`). Quit it and try again.", data_dir.display()))
                    .title("Already running")
                    .blocking_show();
                std::process::exit(1);
            };
            let secrets: Arc<dyn SecretStore> = match std::env::var("SATCHEL_SECRETS").as_deref() {
                Ok("file") => Arc::new(FileSecretStore::new(data_dir.join("secrets.json"))?),
                _ => Arc::new(KeychainSecretStore::new("app.satchel.mail")),
            };
            // Engine workers need a Tokio context; Tauri's runtime is Tokio.
            let engine = tauri::async_runtime::block_on(async {
                Engine::open(EngineOptions {
                    data_dir,
                    mode: Mode::Full,
                    secrets,
                    factory: Arc::new(factory(app.handle())),
                    start_workers: true,
                })
            })?;
            let handle = app.handle().clone();
            let mut rx = engine.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(e) => {
                            let _ = handle.emit("satchel://event", &e);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = handle.emit("satchel://event", serde_json::json!({ "event": "data_changed" }));
                        }
                        Err(_) => break,
                    }
                }
            });
            app.manage(AppState { engine, _lock: lock });
            // Built here, not from the config, so we can catch popups. WebKit
            // runs no listeners in the script-less message iframe, so its links
            // open as `target=_blank` popups; hand them to the UI, which asks
            // first for risky mail and opens them in the system browser.
            let links = app.handle().clone();
            let config = app.config().app.windows.iter().find(|w| w.label == "main").ok_or("no main window in tauri.conf.json")?;
            let window = WebviewWindowBuilder::from_config(app.handle(), config)?
                .on_new_window(move |url, _| {
                    let _ = links.emit_to("main", "satchel://open-link", url.as_str());
                    NewWindowResponse::Deny
                })
                .build()?;
            save_window_state_on_change(&window);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api, open_url, save_file, choose_folder, cli_path])
        .run(tauri::generate_context!())
        .expect("error while running Satchel Mail");
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn saved_files_are_quarantined() {
        let path = std::env::temp_dir().join(format!("satchel-quarantine-{}.command", std::process::id()));
        std::fs::write(&path, "echo hi").unwrap();
        super::mark_downloaded(&path).unwrap();
        let out = std::process::Command::new("xattr").arg("-p").arg("com.apple.quarantine").arg(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        let v = String::from_utf8_lossy(&out.stdout);
        assert!(v.starts_with("0081;") && v.trim_end().ends_with(";Satchel Mail;"), "{v}");
    }
}
