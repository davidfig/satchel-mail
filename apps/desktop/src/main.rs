//! Satchel Mail desktop host: owns the engine, bridges the UI over Tauri IPC
//! and forwards core events. All product logic lives in satchel-core.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use base64::Engine as _;
use satchel_api::{Request, Response};
use satchel_core::secrets::{FileSecretStore, KeychainSecretStore, SecretStore};
use satchel_core::{Caller, Engine, EngineOptions, Mode};
use tauri::{Emitter, Manager, State};
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

/// Ask where to save an attachment, then write it. Returns the chosen path.
#[tauri::command]
async fn save_file(app: tauri::AppHandle, filename: String, data_base64: String) -> Result<Option<String>, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_base64).map_err(|e| e.to_string())?;
    let safe: String = filename.chars().map(|c| if "/\\:".contains(c) { '_' } else { c }).collect();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(&safe).save_file(move |p| {
        let _ = tx.send(p);
    });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
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
        .plugin(tauri_plugin_dialog::init());
    #[cfg(feature = "updater")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
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
                    factory: Arc::new(satchel_providers::Factory::new()),
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api, open_url, save_file, cli_path])
        .run(tauri::generate_context!())
        .expect("error while running Satchel Mail");
}
